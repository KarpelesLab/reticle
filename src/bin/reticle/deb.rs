//! Reading files out of a Debian package, for `reticle fetch icestorm`.
//!
//! A `.deb` is an `ar` archive of three members — `debian-binary`,
//! `control.tar.*` and `data.tar.*` — and the files are in the last, a tar
//! archive compressed with xz. The `ar` and tar halves are a few dozen lines
//! each and are here; the xz half is `compcol`'s, the dependency the
//! `apicula` feature already brings in for Project Apicula's databases, so
//! this one is behind a feature for the same reason (`icestorm`).
//!
//! Only what a Debian package uses is read: `ar`'s common format with
//! 16-byte names, and POSIX ustar entries of regular files. A member or an
//! entry this does not understand is skipped, never guessed at, and a file
//! asked for and not found is an error naming it.

/// The members of an `ar` archive, as `(name, bytes)`.
///
/// # Errors
///
/// A message for a file that is not an `ar` archive or is truncated.
pub(crate) fn ar_members(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    let mut rest = bytes
        .strip_prefix(b"!<arch>\n")
        .ok_or("not an ar archive: no `!<arch>` magic")?;
    let mut out = Vec::new();
    while rest.len() >= 60 {
        let header = &rest[..60];
        let name = String::from_utf8_lossy(&header[..16])
            .trim_end()
            .trim_end_matches('/')
            .to_owned();
        let size: usize = String::from_utf8_lossy(&header[48..58])
            .trim()
            .parse()
            .map_err(|_| format!("a bad size in the ar header of `{name}`"))?;
        let body = rest
            .get(60..60 + size)
            .ok_or_else(|| format!("the ar member `{name}` is truncated"))?;
        out.push((name, body));
        // Members are padded to an even offset.
        let next = 60 + size + size % 2;
        rest = rest.get(next..).unwrap_or(&[]);
    }
    Ok(out)
}

/// The regular files of a ustar archive, as `(path, bytes)`, with a
/// leading `./` removed.
///
/// # Errors
///
/// A message for a truncated archive or a header that does not parse.
pub(crate) fn tar_files(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 512 <= bytes.len() {
        let header = &bytes[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let field = |range: std::ops::Range<usize>| {
            let raw = &header[range];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).into_owned()
        };
        let mut name = field(0..100);
        let prefix = field(345..500);
        if !prefix.is_empty() {
            name = format!("{prefix}/{name}");
        }
        let size_text = field(124..136);
        let size = usize::from_str_radix(size_text.trim(), 8)
            .map_err(|_| format!("a bad size in the tar header of `{name}`"))?;
        let kind = header[156];
        let body = bytes
            .get(at + 512..at + 512 + size)
            .ok_or_else(|| format!("the tar entry `{name}` is truncated"))?;
        if kind == b'0' || kind == 0 {
            out.push((name.trim_start_matches("./").to_owned(), body));
        }
        at += 512 + size.div_ceil(512) * 512;
    }
    Ok(out)
}

/// The files at `paths` inside the package's data archive, in that order.
///
/// # Errors
///
/// A message when the package cannot be read, its data archive is not
/// xz-compressed, this build cannot decompress xz, or a path is missing.
pub(crate) fn data_files(deb: &[u8], paths: &[&str]) -> Result<Vec<Vec<u8>>, String> {
    let members = ar_members(deb)?;
    let (_, data) = members
        .iter()
        .find(|(name, _)| name == "data.tar.xz")
        .ok_or("the package has no data.tar.xz")?;
    let tar = unxz(data)?;
    let files = tar_files(&tar)?;
    paths
        .iter()
        .map(|want| {
            files
                .iter()
                .find(|(path, _)| path == want)
                .map(|(_, bytes)| bytes.to_vec())
                .ok_or_else(|| format!("the package has no `{want}`"))
        })
        .collect()
}

#[cfg(feature = "icestorm")]
fn unxz(bytes: &[u8]) -> Result<Vec<u8>, String> {
    compcol::vec::decompress_to_vec::<compcol::xz::Xz>(bytes)
        .map_err(|err| format!("the data archive does not decompress: {err:?}"))
}

#[cfg(not(feature = "icestorm"))]
fn unxz(_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err(
        "this build cannot unpack an xz-compressed package: the `icestorm` feature is off. \
         Rebuild with `cargo build --features cli,icestorm`, or unpack the package by hand \
         and set RETICLE_ICESTORMDB to the directory holding chipdb-5k.txt"
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ar(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = b"!<arch>\n".to_vec();
        for (name, body) in members {
            let header = format!(
                "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                name,
                0,
                0,
                0,
                "100644",
                body.len()
            );
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(body);
            if body.len() % 2 == 1 {
                out.push(b'\n');
            }
        }
        out
    }

    fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, body) in files {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            let size = format!("{:011o}\0", body.len());
            h[124..136].copy_from_slice(size.as_bytes());
            h[156] = b'0';
            out.extend_from_slice(&h);
            out.extend_from_slice(body);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.extend_from_slice(&[0u8; 1024]);
        out
    }

    /// An odd-length member is padded, and the next one is still found.
    #[test]
    fn ar_members_are_found_past_padding() {
        let bytes = ar(&[
            ("debian-binary", b"2.0\n"),
            ("odd", b"abc"),
            ("data.tar.xz", b"xyz"),
        ]);
        let members = ar_members(&bytes).unwrap();
        let names: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["debian-binary", "odd", "data.tar.xz"]);
        assert_eq!(members[2].1, b"xyz");
        assert!(ar_members(b"not an archive").is_err());
    }

    /// Tar entries are read at 512-byte boundaries, and `./` is dropped.
    #[test]
    fn tar_files_are_read_whole() {
        let bytes = tar(&[("./usr/a.txt", &[1u8; 700]), ("./usr/b.txt", b"hello")]);
        let files = tar_files(&bytes).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "usr/a.txt");
        assert_eq!(files[0].1.len(), 700);
        assert_eq!(files[1].1, b"hello");
    }
}
