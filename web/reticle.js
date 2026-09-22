// reticle.js — a hand-written wrapper over Reticle's WebAssembly surface.
//
// The .wasm module exports plain `extern "C"` functions over linear memory
// (see `src/wasm/mod.rs`); there is no bindgen and no generated glue, so
// this file is the whole JavaScript side. It has no dependencies and runs
// unchanged in a browser or in a module-aware runtime.
//
//   import { Reticle } from './reticle.js';
//   const reticle = await Reticle.load('./reticle.wasm');
//   const { ok, rtl, diagnostics } = reticle.elaborate('verilog', 'top.v', src);
//
// THE PROTOCOL
//
//   A string argument is a (pointer, length) pair of UTF-8 bytes that the
//   caller allocates with `reticle_wasm_alloc` and frees afterwards.
//
//   A result is one pointer to a buffer laid out as a little-endian uint32
//   byte length followed by that many bytes of UTF-8. The caller decodes
//   it and returns the buffer with `reticle_wasm_free(ptr, 4 + len)`. A
//   null pointer means the allocation failed.
//
//   Every operation but `version` and `features` answers with JSON of one
//   shape: { ok, errors, warnings, rendered, diagnostics: [...] } plus the
//   fields that call produces. `rendered` is the rustc-style text with
//   source excerpts; `diagnostics` is the same information as objects.
//
// Nothing here holds a pointer between calls: a design travels between
// steps as `.rtl` text, so there is nothing for a caller to leak.

/** One call's answer. Every method returns an object of this shape. */
const EMPTY_REPLY = Object.freeze({
  ok: false,
  errors: 0,
  warnings: 0,
  rendered: '',
  diagnostics: [],
});

/**
 * Reads a `.wasm` file as bytes, from a URL or a File.
 *
 * `fetch` is blocked for `file://` URLs in most browsers, so the page can
 * also hand over a File the user picked; both paths end in an ArrayBuffer.
 */
async function readWasm(source) {
  if (source instanceof ArrayBuffer) return source;
  if (typeof Blob !== 'undefined' && source instanceof Blob) {
    return await source.arrayBuffer();
  }
  // A URL. Try fetch, then fall back to XMLHttpRequest, which some
  // browsers still allow for a same-directory file:// request.
  try {
    const response = await fetch(source);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return await response.arrayBuffer();
  } catch (fetchError) {
    if (typeof XMLHttpRequest === 'undefined') throw fetchError;
    return await new Promise((resolve, reject) => {
      const request = new XMLHttpRequest();
      request.open('GET', source, true);
      request.responseType = 'arraybuffer';
      request.onload = () =>
        request.response
          ? resolve(request.response)
          : reject(new Error(`could not read ${source}`));
      request.onerror = () =>
        reject(
          new Error(
            `could not read ${source}: ${fetchError.message}. ` +
              'Browsers block reading files over file://; serve the ' +
              'directory over HTTP, or load the .wasm with the file picker.',
          ),
        );
      request.send();
    });
  }
}

/** The compiler, bound to one instantiated WebAssembly module. */
export class Reticle {
  /**
   * Instantiates the module.
   *
   * `source` is a URL, an ArrayBuffer or a File/Blob.
   */
  static async load(source = './reticle.wasm') {
    const bytes = await readWasm(source);
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return new Reticle(instance);
  }

  /** Wraps an already-instantiated module. Prefer {@link Reticle.load}. */
  constructor(instance) {
    this.exports = instance.exports;
    this.encoder = new TextEncoder();
    this.decoder = new TextDecoder('utf-8');
    const required = [
      'reticle_wasm_alloc',
      'reticle_wasm_free',
      'reticle_wasm_version',
      'reticle_wasm_features',
      'reticle_wasm_elaborate',
      'reticle_wasm_synth',
      'reticle_wasm_emit',
      'reticle_wasm_simulate',
    ];
    for (const name of required) {
      if (typeof this.exports[name] !== 'function') {
        throw new Error(`this .wasm does not export ${name}`);
      }
    }
  }

  /** A view of the module's memory, re-read because it may have grown. */
  get bytes() {
    return new Uint8Array(this.exports.memory.buffer);
  }

  /**
   * Copies a string into the module's memory.
   *
   * Returns `[pointer, length]`; the caller must free it. An empty string
   * is `[0, 0]`, which the Rust side reads as "not given".
   */
  alloc(text) {
    const encoded = this.encoder.encode(text ?? '');
    if (encoded.length === 0) return [0, 0];
    const pointer = this.exports.reticle_wasm_alloc(encoded.length);
    if (pointer === 0) throw new Error('out of memory in the wasm module');
    this.bytes.set(encoded, pointer);
    return [pointer, encoded.length];
  }

  /** Releases a buffer from {@link Reticle#alloc}. */
  free(pointer, length) {
    if (pointer !== 0 && length !== 0) {
      this.exports.reticle_wasm_free(pointer, length);
    }
  }

  /** Decodes a length-prefixed result buffer and releases it. */
  take(pointer) {
    if (pointer === 0) throw new Error('the wasm module ran out of memory');
    const memory = this.bytes;
    const length = new DataView(this.exports.memory.buffer).getUint32(pointer, true);
    const text = this.decoder.decode(memory.subarray(pointer + 4, pointer + 4 + length));
    this.exports.reticle_wasm_free(pointer, length + 4);
    return text;
  }

  /** Calls `name` with string arguments and extra scalars, and decodes. */
  call(name, strings, scalars = []) {
    const allocated = strings.map((text) => this.alloc(text));
    try {
      const args = allocated.flat().concat(scalars);
      return this.take(this.exports[name](...args));
    } finally {
      for (const [pointer, length] of allocated) this.free(pointer, length);
    }
  }

  /** Like {@link Reticle#call}, but parses the JSON reply. */
  callJson(name, strings, scalars = []) {
    try {
      return { ...EMPTY_REPLY, ...JSON.parse(this.call(name, strings, scalars)) };
    } catch (error) {
      return {
        ...EMPTY_REPLY,
        errors: 1,
        rendered: `error: ${error.message}\n`,
        diagnostics: [
          {
            severity: 'error',
            code: '',
            message: error.message,
            file: '',
            line: 0,
            column: 0,
            notes: [],
          },
        ],
      };
    }
  }

  /** The library version, such as `"0.1.0"`. */
  version() {
    return this.call('reticle_wasm_version', []);
  }

  /** The stages this build has, as an array of names. */
  features() {
    const text = this.call('reticle_wasm_features', []);
    return text === '' ? [] : text.split(',');
  }

  /** True when `stage` (`verilog`, `vhdl`, `sim`, `synth`) is available. */
  has(stage) {
    return this.features().includes(stage);
  }

  /**
   * Parses and elaborates one source.
   *
   * `language` is `'verilog'` or `'vhdl'`; `top` names the root module or
   * entity and may be omitted. On success the reply carries the design as
   * `.rtl` text in `rtl`, which every other method takes back.
   */
  elaborate(language, name, source, top = '') {
    return this.callJson('reticle_wasm_elaborate', [language, name, source, top]);
  }

  /**
   * Elaborates and reports only the diagnostics: the "check" button.
   *
   * The same call as {@link Reticle#elaborate}; this name exists because
   * that is what the page is asking for.
   */
  check(language, name, source, top = '') {
    return this.elaborate(language, name, source, top);
  }

  /**
   * Synthesises a `.rtl` design.
   *
   * `lutInputs` is 0 for a generic netlist or 2 to 8 to map the leftover
   * logic onto k-input LUTs. The reply has the netlist in `rtl` and the
   * pass log in `report`.
   */
  synth(rtl, lutInputs = 0) {
    return this.callJson('reticle_wasm_synth', [rtl], [lutInputs]);
  }

  /**
   * Renders a `.rtl` design as `verilog`, `vhdl`, `json`, `blif` or
   * `edif`. The text comes back in `output`.
   */
  emit(rtl, format = 'verilog') {
    return this.callJson('reticle_wasm_emit', [rtl, format]);
  }

  /**
   * Simulates a `.rtl` design.
   *
   * `ticks` is how far to run; 0 runs until the queue drains or the design
   * calls `$finish`. The reply has `output` ($display text), `time`,
   * `status` and, with `vcd: true`, the waveform in `vcd`.
   */
  simulate(rtl, { top = '', ticks = 0, vcd = false } = {}) {
    return this.callJson('reticle_wasm_simulate', [rtl, top], [BigInt(ticks), vcd ? 1 : 0]);
  }

  /**
   * Source in, netlist out: elaborate, optionally synthesise, then emit.
   *
   * Stops at the first step that fails and returns that step's reply with
   * a `step` field saying where it stopped, so a caller has one thing to
   * check instead of three.
   */
  compile(language, name, source, { top = '', synth = true, lutInputs = 0, format = 'verilog' } = {}) {
    const elaborated = this.elaborate(language, name, source, top);
    if (!elaborated.ok) return { ...elaborated, step: 'elaborate' };

    let rtl = elaborated.rtl;
    let report = '';
    if (synth) {
      const synthesised = this.synth(rtl, lutInputs);
      if (!synthesised.ok) return { ...synthesised, step: 'synth' };
      rtl = synthesised.rtl;
      report = synthesised.report ?? '';
    }

    const emitted = this.emit(rtl, format);
    if (!emitted.ok) return { ...emitted, step: 'emit' };
    return {
      ...emitted,
      step: 'emit',
      rtl,
      report,
      // Elaboration warnings matter even when everything succeeded.
      diagnostics: elaborated.diagnostics.concat(emitted.diagnostics),
      warnings: elaborated.warnings + emitted.warnings,
      rendered: elaborated.rendered + emitted.rendered,
    };
  }
}

export default Reticle;
