//! Just enough of the zip format to take files out of a Python wheel.
//!
//! Project Apicula's chip databases are published inside `apycula`'s
//! wheel on PyPI, which is a zip archive (PEP 427). `datadir` downloads
//! the wheel, checks it against PyPI's SHA-256, and then needs a handful
//! of members out of it. That is all this does: read the central
//! directory, find a member by name, and inflate it with the DEFLATE
//! decoder the FST reader already carries (`reticle::sim::fst::zlib`).
//!
//! What it refuses rather than guesses at: zip64 archives, encrypted
//! members, and any compression method but *stored* (0) and *deflate*
//! (8). A wheel uses none of the first two and only the last two
//! methods, and each member's CRC-32 and size are checked after
//! extraction, so a reader that got the layout wrong fails loudly.

use reticle::sim::fst::zlib;

/// One member, as the central directory describes it.
#[derive(Debug, Clone)]
pub(crate) struct Member {
    /// The member's path inside the archive, `/`-separated.
    pub(crate) name: String,
    method: u16,
    crc: u32,
    compressed: usize,
    size: usize,
    /// Where the member's local header starts.
    offset: usize,
}

/// End of central directory record.
const EOCD: u32 = 0x0605_4b50;
/// Central directory file header.
const CENTRAL: u32 = 0x0201_4b50;
/// Local file header.
const LOCAL: u32 = 0x0403_4b50;

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn usize_at(data: &[u8], at: usize) -> Option<usize> {
    usize::try_from(u32_at(data, at)?).ok()
}

/// Every member of the archive, in central-directory order.
pub(crate) fn members(data: &[u8]) -> Result<Vec<Member>, String> {
    // The end record is 22 bytes plus a comment of up to 65535, so it is
    // found by scanning back from the end for its signature.
    let floor = data.len().saturating_sub(22 + 0xffff);
    let eocd = (floor..=data.len().saturating_sub(22))
        .rev()
        .find(|&at| u32_at(data, at) == Some(EOCD))
        .ok_or("not a zip archive: no end of central directory record")?;
    let bad = || "a damaged zip central directory".to_owned();
    let count = usize::from(u16_at(data, eocd + 10).ok_or_else(bad)?);
    let mut at = usize_at(data, eocd + 16).ok_or_else(bad)?;
    if count == 0xffff || at == 0xffff_ffff {
        return Err("a zip64 archive, which this reader does not handle".to_owned());
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(data, at) != Some(CENTRAL) {
            return Err(bad());
        }
        let flags = u16_at(data, at + 8).ok_or_else(bad)?;
        let name_len = usize::from(u16_at(data, at + 28).ok_or_else(bad)?);
        let extra_len = usize::from(u16_at(data, at + 30).ok_or_else(bad)?);
        let comment_len = usize::from(u16_at(data, at + 32).ok_or_else(bad)?);
        let name = data.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
        let name = String::from_utf8(name.to_vec()).map_err(|_| bad())?;
        if flags & 1 != 0 {
            return Err(format!("`{name}` is encrypted"));
        }
        out.push(Member {
            method: u16_at(data, at + 10).ok_or_else(bad)?,
            crc: u32_at(data, at + 16).ok_or_else(bad)?,
            compressed: usize_at(data, at + 20).ok_or_else(bad)?,
            size: usize_at(data, at + 24).ok_or_else(bad)?,
            offset: usize_at(data, at + 42).ok_or_else(bad)?,
            name,
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// One member's contents, checked against its recorded size and CRC-32.
pub(crate) fn extract(data: &[u8], member: &Member) -> Result<Vec<u8>, String> {
    let name = &member.name;
    let at = member.offset;
    if u32_at(data, at) != Some(LOCAL) {
        return Err(format!(
            "`{name}`: no local header where the directory says"
        ));
    }
    // The local header's own name and extra field lengths, which need not
    // match the central directory's extra field.
    let skip = |off| u16_at(data, at + off).map(usize::from);
    let (Some(name_len), Some(extra_len)) = (skip(26), skip(28)) else {
        return Err(format!("`{name}`: a truncated local header"));
    };
    let start = at + 30 + name_len + extra_len;
    let raw = data
        .get(start..start + member.compressed)
        .ok_or_else(|| format!("`{name}`: runs past the end of the archive"))?;
    let bytes = match member.method {
        0 => raw.to_vec(),
        8 => zlib::inflate(raw, Some(member.size))
            .ok_or_else(|| format!("`{name}`: the DEFLATE stream does not decode"))?,
        m => return Err(format!("`{name}`: compression method {m} is not handled")),
    };
    if bytes.len() != member.size || zlib::crc32(&bytes) != member.crc {
        return Err(format!("`{name}`: the size or CRC-32 does not match"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{extract, members};
    use reticle::sim::fst::zlib;

    /// A two-member archive written by hand the way `zipfile` writes one:
    /// a stored member and a deflated one, a central directory, an end
    /// record, and a comment for the end-record scan to skip.
    fn archive(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, body, deflate) in files {
            let data = if deflate {
                zlib::deflate(body)
            } else {
                body.to_vec()
            };
            let method: u16 = if deflate { 8 } else { 0 };
            let offset = u32::try_from(out.len()).unwrap();
            let crc = zlib::crc32(body);
            let sizes = |v: &mut Vec<u8>| {
                v.extend(crc.to_le_bytes());
                v.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
                v.extend(u32::try_from(body.len()).unwrap().to_le_bytes());
            };
            let name_len = u16::try_from(name.len()).unwrap().to_le_bytes();
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20, 0, 0, 0]);
            out.extend(method.to_le_bytes());
            out.extend([0; 4]);
            sizes(&mut out);
            out.extend(name_len);
            out.extend([0, 0]);
            out.extend(name.as_bytes());
            out.extend(&data);

            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend(method.to_le_bytes());
            central.extend([0; 4]);
            sizes(&mut central);
            central.extend(name_len);
            central.extend([0; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let at = u32::try_from(out.len()).unwrap();
        let n = u16::try_from(files.len()).unwrap().to_le_bytes();
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0; 4]);
        out.extend(n);
        out.extend(n);
        out.extend(u32::try_from(central.len()).unwrap().to_le_bytes());
        out.extend(at.to_le_bytes());
        out.extend(7u16.to_le_bytes());
        out.extend(b"comment");
        out
    }

    #[test]
    fn a_stored_and_a_deflated_member_come_back_out() {
        let text = b"the same line, over and over. ".repeat(40);
        let zip = archive(&[("a/LICENSE", b"MIT", false), ("a/db.bin", &text, true)]);
        let list = members(&zip).expect("reads");
        let names: Vec<&str> = list.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["a/LICENSE", "a/db.bin"]);
        assert_eq!(extract(&zip, &list[0]).unwrap(), b"MIT");
        assert_eq!(extract(&zip, &list[1]).unwrap(), text);
    }

    #[test]
    fn a_damaged_member_is_refused_rather_than_returned() {
        let mut zip = archive(&[("x", b"hello, world", false)]);
        let list = members(&zip).unwrap();
        // The stored body starts after the 30-byte header and the name.
        zip[31] ^= 1;
        assert!(extract(&zip, &list[0]).unwrap_err().contains("CRC-32"));
        assert!(members(b"not a zip at all").is_err());
    }
}
