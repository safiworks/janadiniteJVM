use proc_macro::TokenStream;

mod opcode_decode;

/// NOTE: This macro was AI generated...
///
///
/// `#[derive(Decode)]` generates the body of `OpCode::decode`, i.e. the big
/// `match op { .. }` that used to be hand-written inside `Instructions::next_op`.
///
/// It reads three things off the enum:
///
/// 1. The variant's own discriminant (`Iload(u8) = 0x15`) — used verbatim as
///    the match arm's pattern.
/// 2. The variant's field types (`u8`, `i8`, `u16`, `i16`) — used to decide
///    how each operand byte is read off the bytecode stream.
/// 3. Three opt-in `#[decode(..)]` directives for everything that can't be
///    expressed as "one opcode, one discriminant":
///
///    - `#[decode(short(base = 0x1a, count = 4))]` on a variant that already
///      has a discriminant and exactly one field — also generates arms for
///      `base..=(base + count - 1)`, where the field value is `op - base`.
///      This is the `iload_<n>` / `istore_<n>` / etc. family.
///
///    - `#[decode(wide(op = 0x15))]` on a variant with *no* discriminant —
///      it's only reachable through the enum-level `wide_prefix` opcode
///      (`0xc4`), keyed by the widened opcode (`op`). This is the `Wide*`
///      family.
///
///    - `#[decode(invalid)]` on the fallback variant (e.g. `Invalid(u8)`),
///      used both as the final wildcard arm and as what `opnd!` returns on
///      a truncated/invalid read.
///
///    - `#[decode(skip)]` excludes a variant from codegen entirely, for the
///      handful of opcodes too irregular to describe declaratively (looking
///      at you, `iconst_<n>`/`bipush`). Combine with the enum-level
///      `fallback = some_fn` attribute to hand those opcodes off to a
///      manually written function with the same signature as `decode`.
///
/// Enum-level attribute: `#[decode(wide_prefix = 0xc4, fallback = custom_decode)]`
/// (`fallback` is optional; without it, unmatched opcodes just become `Invalid(op)`).
#[proc_macro_derive(OpcodeDecode, attributes(decode))]
pub fn derive_opcode_decode(input: TokenStream) -> TokenStream {
    opcode_decode::derive_decode(input)
}
