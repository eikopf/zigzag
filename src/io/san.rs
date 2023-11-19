// SAN is not a single standard (despite the name), but rather
// a set of vague conventions around recording algebraic moves.
//
// This is unfortunate, but this implementation will seek to
// accept as many variants of this notation as is reasonable,
// and to produce a single standardised variant. For now, this
// is limited to the English variant of SAN.
//
// The canonical version of SAN here will be FIDE's, as described in
// appendix C of their Laws of Chess document: https://handbook.fide.com/chapter/E012023

use nom::{
    branch::alt,
    branch::permutation,
    bytes::complete::tag,
    character::complete::{multispace0, one_of},
    combinator::cut,
    combinator::{all_consuming, opt},
    error::VerboseError,
    sequence::{pair, preceded, terminated, tuple, Tuple},
    Finish, IResult, Parser,
};
use thiserror::Error;

use crate::standard::piece::StandardPieceKind;

/// The error returned when attempting to
/// parse an invalid SAN literal.
#[derive(Error, Debug)]
pub enum ParseError<'a> {
    /// Returned if the optional leading character of the literal is invalid
    #[error("Expected one of 'O', 'K', 'Q', 'B', 'R', 'N'; got {0}")]
    InvalidLeadingPiece(char),

    /// Returned if the mandatory target square field is invalid.
    #[error("Expected a valid target square; got {0}")]
    InvalidTargetSquare(&'a str),

    /// Returned if the optional capture field is invalid.
    #[error("Expected one of [x, X, -, :]; got {0}")]
    InvalidCaptureField(char),

    /// Returned if the optional disambiguation field is invalid.
    #[error("Expected a value fulfilling [a-h]?[1-8]?; got {0}")]
    InvalidDisambiguationField(&'a str),

    /// Returned if the optional annotation suffix field is invalid.
    #[error("Expected a value fulfilling [?!]?[?!]?; got {0}")]
    InvalidAnnotationSuffixField(&'a str),

    /// Returned if the optional en passant suffix is invalid.
    #[error("Expected a value equal to \"e.p\"; got {0}")]
    InvalidEnPassantSuffix(&'a str),

    /// Returned if the optional check field is invalid.
    #[error("Expected a value fulfilling [+]?; got {0}")]
    InvalidCheckField(char),

    /// Returned if the optional checkmate field is invalid.
    #[error("Expected a value fulfilling [#]? or [++]?; got {0}")]
    InvalidCheckmateField(char),

    /// Returned if the optional promotion field is invalid.
    #[error("Expected a value fulfilling [=/]?[NBRQ] or ([NBRQ]); got {0}")]
    InvalidPromotionField(&'a str),

    /// Returned if the optional castling field is invalid.
    #[error("Expected either [0O]-[0O] or [0O]-[0O]-[0O]; got {0}")]
    InvalidCastlingField(&'a str),

    /// Returned if the length of the literal is invalid.
    #[error("Expected a literal with at least 2 and at most 12 characters; got {0} characters")]
    InvalidLiteralLength(u8),

    /// Returned if a literal is valid, but then ends in garbage.
    #[error("Got trailing garbage after a valid SAN literal: {0}")]
    TrailingGarbage(&'a str),

    /// Returned if an unknown error occurs while parsing a SAN literal.
    #[error("Failed to parse the provided SAN literal")]
    Unknown,
}

/// Represents the data derived from parsing a
/// valid SAN literal.
///
/// Parsing is provided via the `TryFrom<&'a str>` impl.
#[derive(Debug, PartialEq, Eq)]
pub struct San {
    data: SanData,
    is_check: bool,
    is_checkmate: bool,
    annotation: Option<SuffixAnnotation>,
}

impl<'a> TryFrom<&'a str> for San {
    type Error = VerboseError<&'a str>;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        san_literal(value).finish().map(|(_, san)| san)
    }
}

/// The distinct kinds of data conveyed by a SAN literal.
///
/// Keep in mind that a SAN literal conveys information about
/// a move which may or may not be valid in the context of a
/// given board position. This struct stores the exact data
/// conveyed by the literal, but needs a [`Board`](crate::core::Board) to be converted
/// into a [`Move`](crate::core::Move), and a [`Validate`](crate::core::Validate) to be converted into a
/// [`LegalMove`](crate::core::LegalMove).
#[derive(Debug, Eq, PartialEq)]
enum SanData {
    AbbreviatedPawnMove(AbbreviatedPawnMove),
    CastleMove(CastleMove),
    NormalMove(NormalMove),
    PawnMove(PawnMove),
}

/// Represents a SAN literal denoting a castling move.
///
/// ## Rough Specification
/// Castle moves are entirely described by their direction,
/// and are normally given as 0-0 (kingside) and 0-0-0 (queenside).
/// It's also common to use O instead of 0, and so this should be
/// accepted as an input format.
///
/// While it is impossible to capture while castling, it is
/// theoretically possible to put the opponent both in check
/// and checkmate; hence when parsing a SAN-castle literal you
/// still must check for the common check/checkmate suffixes. As
/// usual, you also want to look for the annotation suffixes as well.
#[derive(Debug, Eq, PartialEq)]
enum CastleMove {
    QueenSide,
    KingSide,
}

/// Represents a SAN literal denoting a normal (non-pawn) move.
///
/// ## Rough Specification
/// Normal moves are always preceded by a capital letter corresponding
/// to the piece, since the absence of this indicates a pawn move. This
/// is optionally followed by a disambiguating field, and then the optional
/// capture indicator. This is followed by a mandatory target square. Finally,
/// we also include the optional check, checkmate, and annotation suffixes.
#[derive(Debug, Eq, PartialEq)]
struct NormalMove {
    piece: StandardPieceKind,
    disambiguation_field: Option<DisambiguationField>,
    target: (char, char),
    is_capture: bool,
}

/// Represents a SAN literal denoting a normal pawn move.
///
/// ## Rough Specification
/// A pawn move is little more than a normal move with no
/// leading character, and which permits an additional
/// promotion piece component.
#[derive(Debug, Eq, PartialEq)]
struct PawnMove {
    target: (char, char),
    is_capture: bool,
    capture_rank: Option<char>,
    promotion_piece: Option<StandardPieceKind>,
}

/// Represents a SAN literal denoting an abbreviated pawn move.
///
/// ## Rough Specification
/// Abbreviated pawn moves are hugely contextual; they are limited
/// to listing just the source and target files, with a capture
/// glyph in between if necessary.
#[derive(Debug, Eq, PartialEq)]
struct AbbreviatedPawnMove {
    source_rank: char,
    target_rank: char,
    is_capture: bool,
    promotion_piece: Option<StandardPieceKind>,
}

/// Describes the optional field
/// used to disambiguate potentially
/// ambiguous moves from one another.
#[derive(Debug, PartialEq, Eq)]
enum DisambiguationField {
    FileLetter(char),
    RankDigit(char),
    SourceSquare((char, char)),
}

/// Describes the traditional
/// suffix annotation used to
/// describe the qualitative
/// aspects of a move.
///
/// Here the word bang corresponds
/// to the exclamation mark (!) and
/// the word hook corresponds to the
/// question mark (?).
#[derive(Debug, PartialEq, Eq)]
enum SuffixAnnotation {
    Bang,     // good move
    Hook,     // mistake
    BangBang, // brilliant move
    BangHook, // interesting move (ambiguous value)
    HookBang, // dubious move (potentially negative value)
    HookHook, // blunder
}

type SanResult<'a, T> = IResult<&'a str, T, VerboseError<&'a str>>;

/// Parses the pattern [?!]?[?!]?.
fn annotation<'a>(source: &'a str) -> SanResult<Option<SuffixAnnotation>> {
    let mut annotation = pair(opt(one_of("!?")), opt(one_of("!?")));
    annotation.parse(source).map(|(tail, pair)| {
        (
            tail,
            match pair {
                (None, None) => None,
                (Some('?'), None) => Some(SuffixAnnotation::Hook),
                (Some('!'), None) => Some(SuffixAnnotation::Bang),
                (Some('?'), Some('?')) => Some(SuffixAnnotation::HookHook),
                (Some('?'), Some('!')) => Some(SuffixAnnotation::HookBang),
                (Some('!'), Some('!')) => Some(SuffixAnnotation::BangBang),
                (Some('!'), Some('?')) => Some(SuffixAnnotation::BangHook),
                _ => unreachable!(),
            },
        )
    })
}

/// Parses the symbol [+] and returns true if it finds it.
fn check<'a>(source: &'a str) -> SanResult<bool> {
    tag("+")
        .parse(source)
        .map(|(tail, symbol)| (tail, symbol == "+"))
}

/// Parses the symbol [#] and returns true if it finds it.
fn checkmate<'a>(source: &'a str) -> SanResult<bool> {
    tag("#")
        .parse(source)
        .map(|(tail, symbol)| (tail, symbol == "#"))
}

/// Parses a piece of the form [KQBNR].
fn piece<'a>(source: &'a str) -> SanResult<StandardPieceKind> {
    one_of("KQBNR").parse(source).map(|(tail, piece)| {
        (
            tail,
            match piece {
                'K' => StandardPieceKind::King,
                'Q' => StandardPieceKind::Queen,
                'B' => StandardPieceKind::Bishop,
                'N' => StandardPieceKind::Knight,
                'R' => StandardPieceKind::Rook,
                _ => unreachable!(),
            },
        )
    })
}

/// Parses a single file character of the form [abcdefgh]
fn file<'a>(source: &'a str) -> SanResult<char> {
    one_of("abcdefgh").parse(source)
}

/// Parses a target position with the form [abcdefgh][12345678].
fn target<'a>(source: &'a str) -> SanResult<(char, char)> {
    let mut target = pair(one_of("abcdefgh"), one_of("12345678"));
    target.parse(source)
}

/// Parses a disambiguation field with the form [abcdefgh]?[12345678]?.
fn disambiguation_field<'a>(source: &'a str) -> SanResult<Option<DisambiguationField>> {
    let mut disambiguation_field = pair(opt(one_of("abcdefgh")), opt(one_of("12345678")));
    disambiguation_field.parse(source).map(|(tail, field)| {
        (
            tail,
            match field {
                (None, None) => None,
                (Some(file), None) => Some(DisambiguationField::FileLetter(file)),
                (None, Some(rank)) => Some(DisambiguationField::RankDigit(rank)),
                (Some(file), Some(rank)) => Some(DisambiguationField::SourceSquare((file, rank))),
                _ => unreachable!(),
            },
        )
    })
}

/// Parses a single capture symbol that can appear in a SAN literal.
fn capture<'a>(source: &'a str) -> SanResult<char> {
    let mut capture = one_of("xX×:");
    capture.parse(source)
}

/// Parses the "=[RNBQ]" segment that can appear at the end of a pawn move.
fn promotion<'a>(source: &'a str) -> SanResult<StandardPieceKind> {
    let promotion_piece = one_of("RNBQ");
    let mut promotion = preceded(tag("="), cut(promotion_piece));
    promotion.parse(source).map(|(tail, piece)| {
        (
            tail,
            match piece {
                'R' => StandardPieceKind::Rook,
                'N' => StandardPieceKind::Knight,
                'B' => StandardPieceKind::Bishop,
                'Q' => StandardPieceKind::Queen,
                _ => unreachable!(),
            },
        )
    })
}

/// Parses a move of the form [abcdefgh][capture]?[abcefgh][promotion]?.
fn abbreviated_pawn_move<'a>(source: &'a str) -> SanResult<SanData> {
    let mut abbrev_move = tuple((
        one_of("abcdefgh"),
        opt(capture),
        one_of("abcdefgh"),
        opt(promotion),
    ));
    abbrev_move
        .parse(source)
        .map(|(tail, (source, capture, target, promotion))| {
            (
                tail,
                SanData::AbbreviatedPawnMove(AbbreviatedPawnMove {
                    source_rank: source,
                    target_rank: target,
                    is_capture: capture.is_some(),
                    promotion_piece: promotion,
                }),
            )
        })
}

/// Parses a pawn move of the form ([abcdefgh]x)?(target)(promotion)?.
fn pawn_move<'a>(source: &'a str) -> SanResult<SanData> {
    let mut pawn_move = tuple((opt(pair(file, capture)), target, opt(promotion)));
    pawn_move
        .parse(source)
        .map(|(tail, (file_capture_block, target, promotion))| {
            (
                tail,
                SanData::PawnMove(PawnMove {
                    target,
                    is_capture: file_capture_block.is_some(),
                    capture_rank: file_capture_block.map(|(file, _)| file),
                    promotion_piece: promotion,
                }),
            )
        })
}

/// Parses a castle move with the form [0O]-[0O](-[0O])?.
fn castle_move<'a>(source: &'a str) -> SanResult<SanData> {
    let mut castle = alt((tag("0-0"), tag("O-O"), tag("0-0-0"), tag("O-O-O")));
    castle.parse(source).map(|(tail, castle)| {
        (
            tail,
            SanData::CastleMove(match castle {
                "O-O" | "0-0" => CastleMove::KingSide,
                "O-O-O" | "0-0-0" => CastleMove::QueenSide,
                _ => unreachable!(),
            }),
        )
    })
}

/// Parses a normal (non-pawn) move with the form [piece][disambiguation_field]?[capture]?[target].
fn normal_move<'a>(source: &'a str) -> SanResult<SanData> {
    let mut normal_move = tuple((piece, disambiguation_field, opt(capture), target));
    normal_move
        .parse(source)
        .map(|(tail, (piece, disambiguation_field, capture, target))| {
            (
                tail,
                SanData::NormalMove(NormalMove {
                    piece,
                    disambiguation_field,
                    target,
                    is_capture: capture.is_some(),
                }),
            )
        })
}

/// Parses a complete SAN literal.
fn san_literal<'a>(source: &'a str) -> SanResult<San> {
    let san_literal = tuple((
        alt((castle_move, abbreviated_pawn_move, pawn_move, normal_move)),
        opt(permutation((check, checkmate))),
        annotation,
    ));

    let mut san_parser = all_consuming(san_literal);
    let (tail, (data, check_state, annotation)) = san_parser.parse(source)?;

    Ok((
        tail,
        San {
            data,
            is_check: check_state.is_some_and(|(check, _)| check),
            is_checkmate: check_state.is_some_and(|(_, checkmate)| checkmate),
            annotation,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_san_parsing() {
        san_literal("e5").unwrap();
        san_literal("Kxd3??").unwrap();
        san_literal("fxg=Q+#!").unwrap();
    }

    #[test]
    fn correct_errors_from_san_parsing() {
        san_literal("fxd=+#!").expect_err("should fail without a promotion piece");
        san_literal("fd!!!").expect_err("should fail because of too many exclamation marks");
        san_literal("").expect_err("should fail because it's empty");
    }

    #[test]
    fn parse_promotion_chunk_correctly() {
        promotion("=Q").expect("should return a StandardPieceKind::Queen.");
        promotion("=").expect_err("should fail with no piece kind");
        promotion("=A").expect_err("should fail");
        promotion("B").expect_err("should fail");
        promotion("").expect_err("should fail");
    }
}
