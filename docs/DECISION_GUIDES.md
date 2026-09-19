# SplitScript decision guides

Use these short guides when the question is which language form owns a task.
Open the linked language or standard-library symbol afterward for exact syntax,
examples, and effects.

## Choose a lifecycle block

<!-- lifecycle-matrix -->

Use [`onAttach`] for cooperative process discovery, then [`onStateReady`] when
initialization needs the first committed [`old`] and [`current`] snapshots.
Use [`whileAttached`] only for per-update bookkeeping that should run before
the timer decisions. [`onStart`] and [`onReset`] observe timer transitions even
while detached, so they cannot use process or snapshot context.

## Choose a state field form

- Use an [`at`](syntax@at) field when polling follows a fixed module-relative,
  absolute, or sibling-field pointer path.
- Use an expression-backed [`state`] field when attachment discovery supplies
  an address, several local steps compute one value, or ordinary API calls read
  the value.
- Keep the field as `T` when a failed read should retain its last accepted
  value. Use [`T?`] only when absence itself should enter [`current`] and
  [`old`].
- Put values in one struct or fixed array when they must succeed and advance as
  one transaction.
- Use independent enum globals for independent build facts. Use one enum when
  each build selects one complete alternative memory shape. Guard conditional
  [`state`] and managed [`class`] fields with those ordinary values.

## Choose absence, failure, retrying, or waiting

- [`T?`] represents expected absence. Match it or use [`else`] when a default is
  appropriate.
- [`T!`] preserves an error. Use postfix [`?`] to transfer it to the nearest
  fallible function, state field, or [`retry`] boundary.
- Use [`else`] for one local fallback that should happen immediately.
- Use [`retry`] for bounded synchronous work that should be evaluated again
  from the beginning on a later attached update.
- Use [`await`] for one existing `async T` operation that owns its polling
  progress. Constructing the future does no work; polling it makes progress.

Do not put [`await`] inside [`retry`]. Await asynchronous discovery first, then
retry a synchronous transaction if the resulting reads can still fail.

## Choose the correct string unit

- [`String`] stores immutable UTF-8 text. Equality compares text, not a host
  language's indexing unit.
- [`byteLength`](method@String.byteLength), [`byteAt`](method@String.byteAt), and
  [`slice`](method@String.slice) use UTF-8 byte offsets. A character may occupy
  several bytes.
- [`charAt`](method@String.charAt) reads the Unicode scalar value beginning at
  a proven UTF-8 character boundary; the supplied index is still a byte offset.
- Native state-field [`utf8`] bounds bytes and rejects invalid UTF-8.
- Native state-field [`utf16le`] bounds two-byte UTF-16 code units and replaces
  unpaired surrogates while decoding.
- Managed [`String`] fields use the UTF-16 length stored in the object header.
  This is an allocation/read bound, not a distinct string type.

When porting C# index arithmetic, first decide whether the source position is a
UTF-16 code-unit index, a Unicode-scalar index, or a byte offset. Do not carry
the number across unchanged until that unit is proven equivalent.
