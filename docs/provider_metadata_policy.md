# Provider Metadata Canonicalization Policy (issue #1249)

This page applies to the `SignalRegistry` contract in
`contracts/signal_registry`. It is enforced by `set_provider_metadata` and
`set_provider_metadata_checked`.

## Principle: reject, never rewrite

The contract stores exactly the bytes it receives. It never trims, folds or
normalizes text on-chain. Instead it accepts only payloads that are already
canonical. As a result:

* the stored value is always the submitted value, so an indexer, a client and
  the contract all see the same bytes;
* two payloads that look the same to a reader cannot both be stored as
  different values, because at most one of them is canonical.

Validation runs before any storage read-modify-write. A rejected payload
leaves metadata, the metadata revision and the event stream unchanged.

The reference implementation is the pure function
`signal_registry::check_metadata_policy(bytes: &[u8])`. Its output depends only
on the input bytes, so clients can run the same check before submitting.

## Rules

Checks run in this order. The first failure is returned, and per-character
rules scan left to right.

| # | Rule | Error (code) |
|---|------|--------------|
| 1 | Payload is 0 bytes | `EmptyMetadata` (6) |
| 2 | Payload is more than `MAX_METADATA_LEN` (256) **bytes** | `MetadataTooLong` (7) |
| 3 | Payload is not well-formed UTF-8: truncated sequence, stray continuation byte, overlong encoding, encoded surrogate, or a value above U+10FFFF | `InvalidMetadataEncoding` (8) |
| 4 | Payload consists only of Unicode `White_Space` characters (space, tab, CR/LF, NBSP, U+3000, U+2028, …) | `EmptyMetadata` (6) |
| 5 | Contains a control character (`Cc`: U+0000–U+001F, U+007F–U+009F, including tab and newline) | `MetadataControlCharacter` (18) |
| 6 | Contains an invisible or misleading code point (list below) | `MetadataDisallowedCharacter` (19) |
| 7 | Contains a code point or pair that is not in canonical NFC form (list below) | `MetadataNotNormalized` (20) |
| 8 | Contains whitespace other than single U+0020 spaces between non-space characters (leading, trailing or repeated spaces, or any other whitespace character) | `MetadataNonCanonicalWhitespace` (21) |

### Rule 6: disallowed code points

* Format and default-ignorable characters: U+00AD soft hyphen, U+0600–U+0605,
  U+061C, U+06DD, U+070F, U+0890–U+0891, U+08E2, U+115F–U+1160 and U+3164 /
  U+FFA0 Hangul fillers, U+17B4–U+17B5, U+180B–U+180F, U+200B–U+200F
  (zero-width characters and LRM/RLM), U+202A–U+202E (bidi
  embeddings/overrides), U+2060–U+206F (word joiner, invisible operators, bidi
  isolates), U+FE00–U+FE0F variation selectors, U+FEFF BOM, U+FFF0–U+FFFB,
  U+110BD, U+110CD, U+13430–U+1343F, U+1BCA0–U+1BCA3, U+1D173–U+1D17A, and
  U+E0000–U+E0FFF (tags and supplementary variation selectors).
* U+FFFC object replacement and U+FFFD replacement character, which signal
  lossy decoding upstream.
* Private use: U+E000–U+F8FF, U+F0000–U+10FFFF.
* Noncharacters: U+FDD0–U+FDEF and U+nFFFE / U+nFFFF in every plane.

Emoji are accepted, but emoji sequences that need U+200D (ZWJ) or U+FE0F
(variation selector 16) are not, because those characters are invisible.

### Rule 7: canonical form

Clients should submit NFC text that uses precomposed characters. The contract
rejects:

* generic combining-mark blocks: U+0300–U+036F, U+1AB0–U+1AFF, U+1DC0–U+1DFF,
  U+20D0–U+20FF, U+FE20–U+FE2F (for example, `e` + U+0301 must be sent as `é`);
* combining kana voicing marks U+3099–U+309A (send `が`, not `か` + U+3099);
* Arabic combining madda and hamza U+0653–U+0655 (send `آ`, not `ا` + U+0653);
* Hangul conjoining jamo U+1100–U+11FF, U+A960–U+A97F, U+D7B0–U+D7FF (send
  precomposed syllables U+AC00–U+D7A3);
* NFC singletons, which normalize to a different code point: U+0374, U+037E,
  U+0387, Greek oxia forms (U+1F71 … U+1FFD), U+2126 OHM SIGN, U+212A KELVIN
  SIGN, U+212B ANGSTROM SIGN, U+2329/U+232A, and CJK compatibility ideographs
  (U+F900–U+FAD9 except the twelve unified ideographs in that block, e.g.
  U+FA0E, plus U+2F800–U+2FA1D);
* NFC composition exclusions: Devanagari, Bengali, Gurmukhi and Oriya nukta
  forms (e.g. U+0958–U+095F), Tibetan precomposed forms, U+2ADC, Hebrew
  presentation forms U+FB1D, U+FB1F, U+FB2A–U+FB4E, and the musical symbols
  U+1D15E–U+1D164 and U+1D1BB–U+1D1C0;
* decomposed BMP vowel-sign and nukta pairs whose NFC form is one precomposed
  code point, in Devanagari, Bengali, Oriya, Tamil, Telugu, Kannada, Malayalam,
  Sinhala, Myanmar and Balinese (for example, Tamil U+0B92 U+0BD7 must be sent
  as U+0B94).

### Known limits

The contract does not carry full Unicode normalization tables, which would
cost too much in WASM size and per-call budget. As a result, these are **not**
detected:

* canonical reordering of several combining marks outside the generic
  combining blocks, for example Hebrew points typed in a non-canonical order;
* compositions in supplementary-plane scripts (Kaithi, Chakma, Grantha,
  Tirhuta, Siddham, and others).

Clients that use these scripts must apply NFC before submitting. Every rule
above still applies to such payloads deterministically.

## Examples

| Input | Result |
|-------|--------|
| `"Alpha Signals"` | Accepted |
| `"Café Zürich"` (precomposed) | Accepted |
| `"東京 シグナル"`, `"가나다"`, `"rocket 🚀 desk"` | Accepted |
| `""`, `"   "`, `"\t"`, `"\u{3000}"` | `EmptyMetadata` |
| `"line\nbreak"`, `"a\tb"`, `"a\u{7F}"` | `MetadataControlCharacter` |
| `"pay\u{200B}pal"`, `"user\u{202E}gnp.exe"`, `"\u{FEFF}bom"` | `MetadataDisallowedCharacter` |
| `"Cafe\u{0301}"`, `"\u{212B}ngstrom"`, `"\u{1100}\u{1161}"` | `MetadataNotNormalized` |
| `" leading"`, `"trailing "`, `"double  space"`, `"a\u{00A0}b"` | `MetadataNonCanonicalWhitespace` |
| 128 × `é` (256 bytes) | Accepted |
| 128 × `é` + `a` (257 bytes) | `MetadataTooLong` |
| bytes `61 E2 82` (truncated `€`) | `InvalidMetadataEncoding` |

## Tests

`contracts/signal_registry/src/lib.rs`: `canonical_metadata_is_accepted`,
`empty_like_metadata_is_rejected`, `control_characters_are_rejected`,
`invisible_and_misleading_characters_are_rejected`,
`non_normalized_characters_are_rejected`, `non_canonical_whitespace_is_rejected`,
`invalid_utf8_is_rejected`, `length_is_counted_in_bytes_at_multibyte_boundaries`,
and `rejected_metadata_never_reaches_storage`. The last one calls both setters
and checks that the stored value, the revision and the event stream are
unchanged after each rejection.
