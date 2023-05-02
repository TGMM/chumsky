//! Token input streams and tools converting to and from them..
//!
//! *“What’s up?” “I don’t know,” said Marvin, “I’ve never been there.”*
//!
//! [`Input`] is the primary trait used to feed input data into a chumsky parser. You can create them in a number of
//! ways: from strings, slices, arrays, etc.

pub use crate::stream::{BoxedExactSizeStream, BoxedStream, Stream};

use super::*;
#[cfg(feature = "memoization")]
use hashbrown::HashMap;

/// A trait for types that represents a stream of input tokens. Unlike [`Iterator`], this type
/// supports backtracking and a few other features required by the crate.
///
/// This trait is sealed and so cannot be implemented by other crates because it has an unstable API. This may
/// eventually change. For now, if you wish to use a type that chumsky does not know about as an input, consider using
/// [`Stream`] or [opening an issue/PR](https://github.com/zesterer/chumsky/issues/new).
pub trait Input<'a>: Sealed + 'a {
    /// The type used to keep track of the current location in the stream
    #[doc(hidden)]
    type Offset: Copy + Hash + Ord + Into<usize>;

    /// The type of singular items read from the stream
    type Token;

    /// The type of a span on this input - to provide custom span context see [`Input::spanned`].
    type Span: Span;

    /// Get the offset representing the start of this stream
    #[doc(hidden)]
    fn start(&self) -> Self::Offset;

    /// The token type returned by [`Input::next_maybe`], allows abstracting over by-value and by-reference inputs.
    #[doc(hidden)]
    type TokenMaybe: Borrow<Self::Token> + Into<MaybeRef<'a, Self::Token>>;

    /// Get the next offset from the provided one, and the next token if it exists
    ///
    /// The token is effectively self-owning (even if it refers to the underlying input) so as to abstract over
    /// by-value and by-reference inputs. For alternatives with stronger guarantees, see [`ValueInput::next`] and
    /// `BorrowInput::next_ref`.
    ///
    /// # Safety
    ///
    /// `offset` must be generated by either `Input::start` or a previous call to this function.
    #[doc(hidden)]
    unsafe fn next_maybe(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::TokenMaybe>);

    /// Create a span from a start and end offset.
    ///
    /// # Safety
    ///
    /// As with [`Input::next_maybe`], the offsets passed to this function must be generated by either [`Input::start`]
    /// or [`Input::next_maybe`].
    #[doc(hidden)]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span;

    // Get the previous offset, saturating at zero
    #[doc(hidden)]
    fn prev(offs: Self::Offset) -> Self::Offset;

    /// Split an input that produces tokens of type `(T, S)` into one that produces tokens of type `T` and spans of
    /// type `S`.
    ///
    /// This is commonly required for lexers that generate token-span tuples. For example, `logos`'
    /// [`SpannedIter`](https://docs.rs/logos/0.12.0/logos/struct.Lexer.html#method.spanned) lexer generates such
    /// pairs.
    ///
    /// Also required is an 'End Of Input' (EoI) span. This span is arbitrary, but is used by the input to produce
    /// sensible spans that extend to the end of the input or are zero-width. Most implementations simply use some
    /// equivalent of `len..len` (i.e: a span where both the start and end offsets are set to the end of the input).
    /// However, what you choose for this span is up to you: but consider that the context, start, and end of the span
    /// will be recombined to create new spans as required by the parser.
    ///
    /// Although `Spanned` does implement [`BorrowInput`], please be aware that, as you might anticipate, the slices
    /// will be those of the original input (usually `&[(T, S)]`) and not `&[T]` so as to avoid the need to copy
    /// around sections of the input.
    fn spanned<T, S>(self, eoi: S) -> SpannedInput<T, S, Self>
    where
        Self: Input<'a, Token = (T, S)> + Sized,
        T: 'a,
        S: Span + Clone + 'a,
    {
        SpannedInput {
            input: self,
            eoi,
            phantom: PhantomData,
        }
    }

    /// Add extra context to spans generated by this input.
    ///
    /// This is useful if you wish to include extra context that applies to all spans emitted during a parse, such as
    /// an identifier that corresponds to the file the spans originated from.
    fn with_context<C>(self, context: C) -> WithContext<C, Self>
    where
        Self: Sized,
        C: Clone,
        Self::Span: Span<Context = ()>,
    {
        WithContext {
            input: self,
            context,
        }
    }
}

/// Implement by inputs that have a known size (including spans)
pub trait ExactSizeInput<'a>: Input<'a> {
    /// Get a span from a start offset to the end of the input.
    #[doc(hidden)]
    unsafe fn span_from(&self, range: RangeFrom<Self::Offset>) -> Self::Span;
}

/// Implemented by inputs that represent slice-like streams of input tokens.
pub trait SliceInput<'a>: ExactSizeInput<'a> {
    /// The unsized slice type of this input. For [`&str`] it's `&str`, and for [`&[T]`] it will be `&[T]`.
    type Slice;

    /// Get a slice from a start and end offset
    // TODO: Make unsafe
    #[doc(hidden)]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice;

    /// Get a slice from a start offset till the end of the input
    // TODO: Make unsafe
    #[doc(hidden)]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice;
}

// Implemented by inputs that reference a string slice and use byte indices as their offset.
/// A trait for types that represent string-like streams of input tokens
pub trait StrInput<'a, C: Char>:
    ValueInput<'a, Offset = usize, Token = C> + SliceInput<'a, Slice = &'a C::Str>
{
}

/// Implemented by inputs that can have tokens borrowed from them.
pub trait ValueInput<'a>: Input<'a> {
    /// Get the next offset from the provided one, and the next token if it exists
    ///
    /// # Safety
    ///
    /// `offset` must be generated by either `Input::start` or a previous call to this function.
    #[doc(hidden)]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>);
}

/// Implemented by inputs that can have tokens borrowed from them.
pub trait BorrowInput<'a>: Input<'a> {
    /// Borrowed version of [`ValueInput::next`] with the same safety requirements.
    ///
    /// # Safety
    ///
    /// Same as [`ValueInput::next`]
    #[doc(hidden)]
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>);
}

impl<'a> Sealed for &'a str {}
impl<'a> Input<'a> for &'a str {
    type Offset = usize;
    type Token = char;
    type Span = SimpleSpan<usize>;

    #[inline]
    fn start(&self) -> Self::Offset {
        0
    }

    type TokenMaybe = char;

    #[inline(always)]
    unsafe fn next_maybe(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::TokenMaybe>) {
        self.next(offset)
    }

    #[inline(always)]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        range.into()
    }

    #[inline(always)]
    fn prev(offs: Self::Offset) -> Self::Offset {
        offs.saturating_sub(1)
    }
}

impl<'a> ExactSizeInput<'a> for &'a str {
    #[inline(always)]
    unsafe fn span_from(&self, range: RangeFrom<Self::Offset>) -> Self::Span {
        (range.start..self.len()).into()
    }
}

impl<'a> ValueInput<'a> for &'a str {
    #[inline(always)]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        if offset < self.len() {
            // SAFETY: `offset < self.len()` above guarantees offset is in-bounds
            //         We only ever return offsets that are at a character boundary
            let c = unsafe {
                self.get_unchecked(offset..)
                    .chars()
                    .next()
                    .unwrap_unchecked()
            };
            (offset + c.len_utf8(), Some(c))
        } else {
            (offset, None)
        }
    }
}

impl<'a> StrInput<'a, char> for &'a str {}

impl<'a> SliceInput<'a> for &'a str {
    type Slice = &'a str;

    #[inline(always)]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        &self[range]
    }

    #[inline(always)]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        &self[from]
    }
}

impl<'a, T> Sealed for &'a [T] {}
impl<'a, T> Input<'a> for &'a [T] {
    type Offset = usize;
    type Token = T;
    type Span = SimpleSpan<usize>;

    #[inline(always)]
    fn start(&self) -> Self::Offset {
        0
    }

    type TokenMaybe = &'a T;

    #[inline(always)]
    unsafe fn next_maybe(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::TokenMaybe>) {
        self.next_ref(offset)
    }

    #[inline(always)]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        range.into()
    }

    #[inline(always)]
    fn prev(offs: Self::Offset) -> Self::Offset {
        offs.saturating_sub(1)
    }
}

impl<'a, T> ExactSizeInput<'a> for &'a [T] {
    #[inline(always)]
    unsafe fn span_from(&self, range: RangeFrom<Self::Offset>) -> Self::Span {
        (range.start..self.len()).into()
    }
}

impl<'a> StrInput<'a, u8> for &'a [u8] {}

impl<'a, T> SliceInput<'a> for &'a [T] {
    type Slice = &'a [T];

    #[inline(always)]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        &self[range]
    }

    #[inline(always)]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        &self[from]
    }
}

impl<'a, T: Clone> ValueInput<'a> for &'a [T] {
    #[inline(always)]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        if let Some(tok) = self.get(offset) {
            (offset + 1, Some(tok.clone()))
        } else {
            (offset, None)
        }
    }
}

impl<'a, T> BorrowInput<'a> for &'a [T] {
    #[inline(always)]
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>) {
        if let Some(tok) = self.get(offset) {
            (offset + 1, Some(tok))
        } else {
            (offset, None)
        }
    }
}

impl<'a, T: 'a, const N: usize> Sealed for &'a [T; N] {}
impl<'a, T: 'a, const N: usize> Input<'a> for &'a [T; N] {
    type Offset = usize;
    type Token = T;
    type Span = SimpleSpan<usize>;

    #[inline(always)]
    fn start(&self) -> Self::Offset {
        0
    }

    type TokenMaybe = &'a T;

    #[inline(always)]
    unsafe fn next_maybe(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::TokenMaybe>) {
        self.next_ref(offset)
    }

    #[inline(always)]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        range.into()
    }

    #[inline(always)]
    fn prev(offs: Self::Offset) -> Self::Offset {
        offs.saturating_sub(1)
    }
}

impl<'a, T: 'a, const N: usize> ExactSizeInput<'a> for &'a [T; N] {
    #[inline(always)]
    unsafe fn span_from(&self, range: RangeFrom<Self::Offset>) -> Self::Span {
        (range.start..N).into()
    }
}

impl<'a, const N: usize> StrInput<'a, u8> for &'a [u8; N] {}

impl<'a, T: 'a, const N: usize> SliceInput<'a> for &'a [T; N] {
    type Slice = &'a [T];

    #[inline(always)]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        &self[range]
    }

    #[inline(always)]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        &self[from]
    }
}

impl<'a, T: Clone + 'a, const N: usize> ValueInput<'a> for &'a [T; N] {
    #[inline(always)]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        if let Some(tok) = self.get(offset) {
            (offset + 1, Some(tok.clone()))
        } else {
            (offset, None)
        }
    }
}

impl<'a, T: 'a, const N: usize> BorrowInput<'a> for &'a [T; N] {
    #[inline(always)]
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>) {
        if let Some(tok) = self.get(offset) {
            (offset + 1, Some(tok))
        } else {
            (offset, None)
        }
    }
}

/// A wrapper around an input that splits an input into spans and tokens. See [`Input::spanned`].
#[derive(Copy, Clone)]
pub struct SpannedInput<T, S, I> {
    input: I,
    eoi: S,
    phantom: PhantomData<T>,
}

/// Utility type required to allow [`SpannedInput`] to implement [`Input`].
#[doc(hidden)]
pub struct SpannedTokenMaybe<'a, I: Input<'a>, T, S>(I::TokenMaybe, PhantomData<(T, S)>);

impl<'a, I: Input<'a, Token = (T, S)>, T, S> Borrow<T> for SpannedTokenMaybe<'a, I, T, S> {
    #[inline(always)]
    fn borrow(&self) -> &T {
        &self.0.borrow().0
    }
}

impl<'a, I: Input<'a, Token = (T, S)>, T, S: 'a> From<SpannedTokenMaybe<'a, I, T, S>>
    for MaybeRef<'a, T>
{
    #[inline(always)]
    fn from(st: SpannedTokenMaybe<'a, I, T, S>) -> MaybeRef<'a, T> {
        match st.0.into() {
            MaybeRef::Ref((tok, _)) => MaybeRef::Ref(tok),
            MaybeRef::Val((tok, _)) => MaybeRef::Val(tok),
        }
    }
}

impl<'a, T, S, I: Input<'a>> Sealed for SpannedInput<T, S, I> {}
impl<'a, T, S, I> Input<'a> for SpannedInput<T, S, I>
where
    I: Input<'a, Token = (T, S)>,
    T: 'a,
    S: Span + Clone + 'a,
{
    type Offset = I::Offset;
    type Token = T;
    type Span = S;

    #[inline(always)]
    fn start(&self) -> Self::Offset {
        self.input.start()
    }

    type TokenMaybe = SpannedTokenMaybe<'a, I, T, S>;

    #[inline(always)]
    unsafe fn next_maybe(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::TokenMaybe>) {
        let (offset, tok) = self.input.next_maybe(offset);
        (offset, tok.map(|tok| SpannedTokenMaybe(tok, PhantomData)))
    }

    #[inline(always)]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        let start = self
            .input
            .next_maybe(range.start)
            .1
            .map_or(self.eoi.start(), |tok| tok.borrow().1.start());
        let end = self
            .input
            .next_maybe(I::prev(range.end))
            .1
            .map_or(self.eoi.start(), |tok| tok.borrow().1.end());
        S::new(self.eoi.context(), start..end)
    }

    #[inline(always)]
    fn prev(offs: Self::Offset) -> Self::Offset {
        I::prev(offs)
    }
}

impl<'a, T, S, I> ExactSizeInput<'a> for SpannedInput<T, S, I>
where
    I: ExactSizeInput<'a, Token = (T, S)>,
    T: 'a,
    S: Span + Clone + 'a,
{
    #[inline(always)]
    unsafe fn span_from(&self, range: RangeFrom<Self::Offset>) -> Self::Span {
        let start = self
            .input
            .next_maybe(range.start)
            .1
            .map_or(self.eoi.start(), |tok| tok.borrow().1.start());
        S::new(self.eoi.context(), start..self.eoi.start())
    }
}

impl<'a, T, S, I> ValueInput<'a> for SpannedInput<T, S, I>
where
    I: ValueInput<'a, Token = (T, S)>,
    T: 'a,
    S: Span + Clone + 'a,
{
    #[inline(always)]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        let (offs, tok) = self.input.next(offset);
        (offs, tok.map(|(tok, _)| tok))
    }
}

impl<'a, T, S, I> BorrowInput<'a> for SpannedInput<T, S, I>
where
    I: Input<'a> + BorrowInput<'a, Token = (T, S)>,
    T: 'a,
    S: Span + Clone + 'a,
{
    #[inline(always)]
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>) {
        let (offs, tok) = self.input.next_ref(offset);
        (offs, tok.map(|(tok, _)| tok))
    }
}

impl<'a, T, S, I> SliceInput<'a> for SpannedInput<T, S, I>
where
    I: Input<'a> + SliceInput<'a, Token = (T, S)>,
    T: 'a,
    S: Span + Clone + 'a,
{
    type Slice = I::Slice;

    #[inline(always)]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        <I as SliceInput>::slice(&self.input, range)
    }

    #[inline(always)]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        <I as SliceInput>::slice_from(&self.input, from)
    }
}

/// An input wrapper contains a user-defined context in its span, in addition to the span of the wrapped input. See
/// [`Input::with_context`].
#[derive(Copy, Clone)]
pub struct WithContext<Ctx, I> {
    input: I,
    context: Ctx,
}

impl<Ctx, I> Sealed for WithContext<Ctx, I> {}
impl<'a, Ctx: Clone + 'a, I: Input<'a>> Input<'a> for WithContext<Ctx, I>
where
    I::Span: Span<Context = ()>,
{
    type Offset = I::Offset;
    type Token = I::Token;
    type Span = (Ctx, I::Span);

    #[inline(always)]
    fn start(&self) -> Self::Offset {
        self.input.start()
    }

    type TokenMaybe = I::TokenMaybe;

    #[inline(always)]
    unsafe fn next_maybe(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::TokenMaybe>) {
        self.input.next_maybe(offset)
    }

    #[inline(always)]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        (self.context.clone(), self.input.span(range))
    }

    #[inline(always)]
    fn prev(offs: Self::Offset) -> Self::Offset {
        I::prev(offs)
    }
}

impl<'a, Ctx: Clone + 'a, I: Input<'a>> ExactSizeInput<'a> for WithContext<Ctx, I>
where
    I: ExactSizeInput<'a>,
    I::Span: Span<Context = ()>,
{
    #[inline(always)]
    unsafe fn span_from(&self, range: RangeFrom<Self::Offset>) -> Self::Span {
        (self.context.clone(), self.input.span_from(range))
    }
}

impl<'a, Ctx: Clone + 'a, I: ValueInput<'a>> ValueInput<'a> for WithContext<Ctx, I>
where
    I::Span: Span<Context = ()>,
{
    #[inline(always)]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        self.input.next(offset)
    }
}

impl<'a, Ctx: Clone + 'a, I: BorrowInput<'a>> BorrowInput<'a> for WithContext<Ctx, I>
where
    I::Span: Span<Context = ()>,
{
    #[inline(always)]
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>) {
        self.input.next_ref(offset)
    }
}

impl<'a, Ctx: Clone + 'a, I: SliceInput<'a>> SliceInput<'a> for WithContext<Ctx, I>
where
    I::Span: Span<Context = ()>,
{
    type Slice = I::Slice;

    #[inline(always)]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        <I as SliceInput>::slice(&self.input, range)
    }

    #[inline(always)]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        <I as SliceInput>::slice_from(&self.input, from)
    }
}

impl<'a, Ctx, C, I> StrInput<'a, C> for WithContext<Ctx, I>
where
    I: StrInput<'a, C>,
    I::Span: Span<Context = ()>,
    Ctx: Clone + 'a,
    C: Char,
{
}

/// Represents a location in an input that can be rewound to.
///
/// Markers can be created with [`InputRef::save`] and rewound to with [`InputRef::rewind`].
pub struct Marker<'a, 'parse, I: Input<'a>> {
    pub(crate) offset: I::Offset,
    pub(crate) err_count: usize,
    phantom: PhantomData<fn(&'parse ()) -> &'parse ()>, // Invariance
}

impl<'a, 'parse, I: Input<'a>> Marker<'a, 'parse, I> {
    /// Get the [`Offset`] that this marker corresponds to.
    pub fn offset(self) -> Offset<'a, 'parse, I> {
        Offset {
            offset: self.offset,
            phantom: PhantomData,
        }
    }
}

impl<'a, 'parse, I: Input<'a>> Copy for Marker<'a, 'parse, I> {}
impl<'a, 'parse, I: Input<'a>> Clone for Marker<'a, 'parse, I> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

/// Represents a location in an input.
///
/// If you to rewind to an old input location, see [`Marker`].
pub struct Offset<'a, 'parse, I: Input<'a>> {
    pub(crate) offset: I::Offset,
    phantom: PhantomData<fn(&'parse ()) -> &'parse ()>, // Invariance
}

impl<'a, 'parse, I: Input<'a>> Copy for Offset<'a, 'parse, I> {}
impl<'a, 'parse, I: Input<'a>> Clone for Offset<'a, 'parse, I> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, 'parse, I: Input<'a>> PartialEq for Offset<'a, 'parse, I> {
    fn eq(&self, other: &Self) -> bool {
        self.offset == other.offset
    }
}

pub(crate) struct Errors<T, E> {
    pub(crate) alt: Option<Located<T, E>>,
    pub(crate) secondary: Vec<Located<T, E>>,
}

impl<T, E> Errors<T, E> {
    /// Returns a slice of the secondary errors (if any) have been emitted since the given marker was created.
    #[inline]
    pub(crate) fn secondary_errors_since(&mut self, err_count: usize) -> &mut [Located<T, E>] {
        self.secondary.get_mut(err_count..).unwrap_or(&mut [])
    }
}

impl<T, E> Default for Errors<T, E> {
    fn default() -> Self {
        Self {
            alt: None,
            secondary: Vec::new(),
        }
    }
}

/// Internal type representing the owned parts of an input - used at the top level by a call to
/// `parse`.
pub(crate) struct InputOwn<'a, 's, I: Input<'a>, E: ParserExtra<'a, I>> {
    pub(crate) input: I,
    pub(crate) errors: Errors<I::Offset, E::Error>,
    pub(crate) state: MaybeMut<'s, E::State>,
    pub(crate) ctx: E::Context,
    #[cfg(feature = "memoization")]
    pub(crate) memos: HashMap<(I::Offset, usize), Option<Located<I::Offset, E::Error>>>,
}

impl<'a, 's, I, E> InputOwn<'a, 's, I, E>
where
    I: Input<'a>,
    E: ParserExtra<'a, I>,
{
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new(input: I) -> InputOwn<'a, 's, I, E>
    where
        E::State: Default,
        E::Context: Default,
    {
        InputOwn {
            input,
            errors: Errors::default(),
            state: MaybeMut::Val(E::State::default()),
            ctx: E::Context::default(),
            #[cfg(feature = "memoization")]
            memos: HashMap::default(),
        }
    }

    pub(crate) fn new_state(input: I, state: &'s mut E::State) -> InputOwn<'a, 's, I, E>
    where
        E::Context: Default,
    {
        InputOwn {
            input,
            errors: Errors::default(),
            state: MaybeMut::Ref(state),
            ctx: E::Context::default(),
            #[cfg(feature = "memoization")]
            memos: HashMap::default(),
        }
    }

    pub(crate) fn as_ref_start<'parse>(&'parse mut self) -> InputRef<'a, 'parse, I, E> {
        InputRef {
            offset: self.input.start(),
            input: &self.input,
            errors: &mut self.errors,
            state: &mut self.state,
            ctx: &self.ctx,
            #[cfg(feature = "memoization")]
            memos: &mut self.memos,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_ref_at<'parse>(
        &'parse mut self,
        offset: I::Offset,
    ) -> InputRef<'a, 'parse, I, E> {
        InputRef {
            offset,
            input: &self.input,
            errors: &mut self.errors,
            state: &mut self.state,
            ctx: &self.ctx,
            #[cfg(feature = "memoization")]
            memos: &mut self.memos,
        }
    }

    pub(crate) fn into_errs(self) -> Vec<E::Error> {
        self.errors
            .secondary
            .into_iter()
            .map(|err| err.err)
            .collect()
    }
}

/// Internal type representing an input as well as all the necessary context for parsing.
pub struct InputRef<'a, 'parse, I: Input<'a>, E: ParserExtra<'a, I>> {
    pub(crate) offset: I::Offset,
    pub(crate) input: &'parse I,
    pub(crate) errors: &'parse mut Errors<I::Offset, E::Error>,
    pub(crate) state: &'parse mut E::State,
    pub(crate) ctx: &'parse E::Context,
    #[cfg(feature = "memoization")]
    pub(crate) memos: &'parse mut HashMap<(I::Offset, usize), Option<Located<I::Offset, E::Error>>>,
}

impl<'a, 'parse, I: Input<'a>, E: ParserExtra<'a, I>> InputRef<'a, 'parse, I, E> {
    #[inline]
    pub(crate) fn with_ctx<'sub_parse, C, O>(
        &'sub_parse mut self,
        new_ctx: &'sub_parse C,
        f: impl FnOnce(&mut InputRef<'a, 'sub_parse, I, extra::Full<E::Error, E::State, C>>) -> O,
    ) -> O
    where
        'parse: 'sub_parse,
        C: 'a,
    {
        let mut new_inp = InputRef {
            input: self.input,
            offset: self.offset,
            state: self.state,
            ctx: new_ctx,
            errors: self.errors,
            #[cfg(feature = "memoization")]
            memos: self.memos,
        };
        let res = f(&mut new_inp);
        self.offset = new_inp.offset;
        res
    }

    #[inline]
    pub(crate) fn with_input<'sub_parse, O>(
        &'sub_parse mut self,
        new_input: &'sub_parse I,
        f: impl FnOnce(&mut InputRef<'a, 'sub_parse, I, E>) -> O,
        #[cfg(feature = "memoization")] memos: &'sub_parse mut HashMap<
            (I::Offset, usize),
            Option<Located<I::Offset, E::Error>>,
        >,
    ) -> O
    where
        'parse: 'sub_parse,
    {
        let mut new_inp = InputRef {
            offset: new_input.start(),
            input: new_input,
            state: self.state,
            ctx: self.ctx,
            errors: self.errors,
            #[cfg(feature = "memoization")]
            memos,
        };
        f(&mut new_inp)
    }

    /// Get the internal offset of the input at this moment in time.
    ///
    /// Can be used for generating spans or slices. See [`InputRef::span`] and [`InputRef::slice`].
    #[inline(always)]
    pub fn offset(&self) -> Offset<'a, 'parse, I> {
        Offset {
            offset: self.offset,
            phantom: PhantomData,
        }
    }

    /// Save the current parse state as a [`Marker`].
    ///
    /// You can rewind back to this state later with [`InputRef::rewind`].
    #[inline(always)]
    pub fn save(&self) -> Marker<'a, 'parse, I> {
        Marker {
            offset: self.offset,
            err_count: self.errors.secondary.len(),
            phantom: PhantomData,
        }
    }

    /// Reset the parse state to that represented by the given [`Marker`].
    ///
    /// You can create a marker with which to perform rewinding using [`InputRef::save`].
    #[inline(always)]
    pub fn rewind(&mut self, marker: Marker<'a, 'parse, I>) {
        self.errors.secondary.truncate(marker.err_count);
        self.offset = marker.offset;
    }

    /// Get a mutable reference to the state associated with the current parse.
    #[inline(always)]
    pub fn state(&mut self) -> &mut E::State {
        self.state
    }

    /// Get a reference to the context fed to the current parser.
    ///
    /// See [`ConfigParser::configure`] and [`Parser::then_with_ctx`] for more information about context-sensitive
    /// parsing.
    #[inline(always)]
    pub fn ctx(&self) -> &E::Context {
        self.ctx
    }

    #[inline]
    pub(crate) fn skip_while<F: FnMut(&I::Token) -> bool>(&mut self, mut f: F)
    where
        I: ValueInput<'a>,
    {
        loop {
            // SAFETY: offset was generated by previous call to `Input::next`
            let (offset, token) = unsafe { self.input.next(self.offset) };
            if token.filter(&mut f).is_none() {
                break;
            } else {
                self.offset = offset;
            }
        }
    }

    #[inline(always)]
    pub(crate) fn next_inner(&mut self) -> (I::Offset, Option<I::Token>)
    where
        I: ValueInput<'a>,
    {
        // SAFETY: offset was generated by previous call to `Input::next`
        let (offset, token) = unsafe { self.input.next(self.offset) };
        self.offset = offset;
        (self.offset, token)
    }

    #[inline(always)]
    pub(crate) fn next_maybe_inner(&mut self) -> (I::Offset, Option<I::TokenMaybe>) {
        // SAFETY: offset was generated by previous call to `Input::next`
        let (offset, token) = unsafe { self.input.next_maybe(self.offset) };
        self.offset = offset;
        (self.offset, token)
    }

    #[inline(always)]
    pub(crate) fn next_ref_inner(&mut self) -> (I::Offset, Option<&'a I::Token>)
    where
        I: BorrowInput<'a>,
    {
        // SAFETY: offset was generated by previous call to `Input::next`
        let (offset, token) = unsafe { self.input.next_ref(self.offset) };
        self.offset = offset;
        (self.offset, token)
    }

    /// Attempt to parse this input using the given parser.
    ///
    /// # Important Notice
    ///
    /// Parsers that return `Err(...)` are permitted to leave the input in an **unspecified** (but not
    /// [undefined](https://en.wikipedia.org/wiki/Undefined_behavior)) state.
    ///
    /// The only well-specified action you are permitted to perform on the input after an error has occurred is
    /// rewinding to a marker created *before* the error occurred via [`InputRef::rewind`].
    ///
    /// This state is not consistent between releases of chumsky, compilations of the final binary, or even invocations
    /// of the parser. You should not rely on this state for anything, and choosing to rely on it means that your
    /// parser may break in unexpected ways at any time.
    ///
    /// You have been warned.
    pub fn parse<O, P: Parser<'a, I, O, E>>(&mut self, parser: P) -> Result<O, E::Error> {
        match parser.go::<Emit>(self) {
            Ok(out) => Ok(out),
            Err(()) => Err(self.errors.alt.take().expect("error but no alt?").err),
        }
    }

    /// A check-only version of [`InputRef::parse`].
    ///
    /// # Import Notice
    ///
    /// See [`InputRef::parse`] about unspecified behaviour associated with this function.
    pub fn check<O, P: Parser<'a, I, O, E>>(&mut self, parser: P) -> Result<(), E::Error> {
        match parser.go::<Check>(self) {
            Ok(()) => Ok(()),
            Err(()) => Err(self.errors.alt.take().expect("error but no alt?").err),
        }
    }

    /// Get the next token in the input. Returns `None` if the end of the input has been reached.
    ///
    /// This function is more flexible than either [`InputRef::next`] or [`InputRef::next_ref`] since it
    /// only requires that the [`Input`] trait be implemented for `I` (instead of either [`ValueInput`] or
    /// [`BorrowInput`]). However, that increased flexibility for the end user comes with a tradeoff for the
    /// implementation: this function returns a [`MaybeRef<I::Token>`] that provides only a temporary reference to the
    /// token.
    ///
    /// See [`InputRef::next_ref`] if you want get a reference to the next token instead.
    #[inline(always)]
    pub fn next_maybe(&mut self) -> Option<MaybeRef<'a, I::Token>> {
        self.next_maybe_inner().1.map(Into::into)
    }

    /// Get the next token in the input by value. Returns `None` if the end of the input has been reached.
    ///
    /// See [`InputRef::next_ref`] if you want get a reference to the next token instead.
    #[inline(always)]
    pub fn next(&mut self) -> Option<I::Token>
    where
        I: ValueInput<'a>,
    {
        self.next_inner().1
    }

    /// Get a reference to the next token in the input. Returns `None` if the end of the input has been reached.
    ///
    /// See [`InputRef::next`] if you want get the next token by value instead.
    #[inline(always)]
    pub fn next_ref(&mut self) -> Option<&'a I::Token>
    where
        I: BorrowInput<'a>,
    {
        self.next_ref_inner().1
    }

    /// Peek the next token in the input. Returns `None` if the end of the input has been reached.
    ///
    /// See [`InputRef::next_maybe`] for more information about what this function guarantees.
    #[inline(always)]
    pub fn peek_maybe(&self) -> Option<MaybeRef<'a, I::Token>> {
        // SAFETY: offset was generated by previous call to `Input::next`
        unsafe { self.input.next_maybe(self.offset).1.map(Into::into) }
    }

    /// Peek the next token in the input. Returns `None` if the end of the input has been reached.
    #[inline(always)]
    pub fn peek(&self) -> Option<I::Token>
    where
        I: ValueInput<'a>,
    {
        // SAFETY: offset was generated by previous call to `Input::next`
        unsafe { self.input.next(self.offset).1 }
    }

    /// Peek the next token in the input. Returns `None` if the end of the input has been reached.
    #[inline(always)]
    pub fn peek_ref(&self) -> Option<&'a I::Token>
    where
        I: BorrowInput<'a>,
    {
        // SAFETY: offset was generated by previous call to `Input::next`
        unsafe { self.input.next_ref(self.offset).1 }
    }

    /// Skip the next token in the input.
    #[inline(always)]
    pub fn skip(&mut self)
    where
        I: ValueInput<'a>,
    {
        let _ = self.next_inner();
    }

    /// Get a slice of the input that covers the given offset range.
    #[inline]
    pub fn slice(&self, range: Range<Offset<'a, 'parse, I>>) -> I::Slice
    where
        I: SliceInput<'a>,
    {
        self.slice_inner(range.start.offset..range.end.offset)
    }

    /// Get a slice of the input that covers the given offset range.
    #[inline]
    pub fn slice_from(&self, range: RangeFrom<Offset<'a, 'parse, I>>) -> I::Slice
    where
        I: SliceInput<'a>,
    {
        self.slice_from_inner(range.start.offset..)
    }

    // TODO: Unofy with `InputRef::slice`
    #[inline(always)]
    pub(crate) fn slice_inner(&self, range: Range<I::Offset>) -> I::Slice
    where
        I: SliceInput<'a>,
    {
        self.input.slice(range)
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub(crate) fn slice_from_inner(&self, range: RangeFrom<I::Offset>) -> I::Slice
    where
        I: SliceInput<'a>,
    {
        self.input.slice_from(range)
    }

    #[cfg_attr(not(feature = "regex"), allow(dead_code))]
    #[inline(always)]
    pub(crate) fn slice_trailing_inner(&self) -> I::Slice
    where
        I: SliceInput<'a>,
    {
        self.input.slice_from(self.offset..)
    }

    /// Get a span over the input that covers the given offset range.
    #[inline(always)]
    pub fn span(&self, range: Range<Offset<'a, 'parse, I>>) -> I::Span {
        // SAFETY: `Offset` is invariant over 'parse, so we know that this offset came from the same input
        // See `https://plv.mpi-sws.org/rustbelt/ghostcell/`
        unsafe { self.input.span(range.start.offset..range.end.offset) }
    }

    /// Get a span over the input that covers the given offset range.
    // TODO: Unofy with `InputRef::span`
    #[inline(always)]
    pub fn span_from(&self, range: RangeFrom<Offset<'a, 'parse, I>>) -> I::Span
    where
        I: ExactSizeInput<'a>,
    {
        // SAFETY: `Offset` is invariant over 'parse, so we know that this offset came from the same input
        // See `https://plv.mpi-sws.org/rustbelt/ghostcell/`
        unsafe { self.input.span_from(range.start.offset..) }
    }

    /// Generate a span that extends from the provided [`Offset`] to the current input position.
    #[inline(always)]
    pub fn span_since(&self, before: Offset<'a, 'parse, I>) -> I::Span {
        // SAFETY: `Offset` is invariant over 'parse, so we know that this offset came from the same input
        // See `https://plv.mpi-sws.org/rustbelt/ghostcell/`
        unsafe { self.input.span(before.offset..self.offset) }
    }

    #[cfg(feature = "regex")]
    #[inline(always)]
    pub(crate) fn skip_bytes<C>(&mut self, skip: usize)
    where
        C: Char,
        I: StrInput<'a, C>,
    {
        self.offset += skip;
    }

    #[inline]
    pub(crate) fn emit(&mut self, pos: I::Offset, error: E::Error) {
        self.errors.secondary.push(Located::at(pos, error));
    }

    #[inline]
    pub(crate) fn add_alt<Exp: IntoIterator<Item = Option<MaybeRef<'a, I::Token>>>>(
        &mut self,
        at: I::Offset,
        expected: Exp,
        found: Option<MaybeRef<'a, I::Token>>,
        span: I::Span,
    ) {
        // Prioritize errors before choosing whether to generate the alt (avoids unnecessary error creation)
        self.errors.alt = Some(match self.errors.alt.take() {
            Some(alt) => match alt.pos.into().cmp(&at.into()) {
                Ordering::Equal => {
                    if found.is_none() {
                        Located::at(
                            alt.pos,
                            alt.err.replace_expected_found(expected, found, span),
                        )
                    } else {
                        Located::at(alt.pos, alt.err.merge_expected_found(expected, found, span))
                    }
                }
                Ordering::Greater => alt,
                Ordering::Less => {
                    Located::at(at, alt.err.replace_expected_found(expected, found, span))
                }
            },
            None => Located::at(at, Error::expected_found(expected, found, span)),
        });
    }

    #[inline]
    pub(crate) fn add_alt_err(&mut self, at: I::Offset, err: E::Error) {
        // Prioritize errors
        self.errors.alt = Some(match self.errors.alt.take() {
            Some(alt) => match alt.pos.into().cmp(&at.into()) {
                Ordering::Equal => Located::at(alt.pos, alt.err.merge(err)),
                Ordering::Greater => alt,
                Ordering::Less => Located::at(at, err),
            },
            None => Located::at(at, err),
        });
    }
}

/// Struct used in [`Parser::validate`] to collect user-emitted errors
pub struct Emitter<E> {
    emitted: Vec<E>,
}

impl<E> Emitter<E> {
    #[inline]
    pub(crate) fn new() -> Emitter<E> {
        Emitter {
            emitted: Vec::new(),
        }
    }

    #[inline]
    pub(crate) fn errors(self) -> Vec<E> {
        self.emitted
    }

    /// Emit a non-fatal error
    #[inline]
    pub fn emit(&mut self, err: E) {
        self.emitted.push(err)
    }
}
