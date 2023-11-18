//! Traits for representing chessboards.

use super::index::Index;
use super::piece::Piece;
use super::r#move::{IllegalMoveError, LegalMove, Move};

/// Represents a static view into a single board position, with
/// no notion of moves or move legality.
///
/// For a notion of legality see [`Validate`].
///
/// For a notion of moves acting on state, see [`Process`].
pub trait Board: std::fmt::Debug {
    /// Represents a specific place on the board.
    type Index: Index;

    /// Represents the pieces which may be on the board.
    type Piece: Piece;

    /// Returns the piece at the given index by reference
    /// if it exists, otherwise returns none.
    fn get_piece_at(&self, index: Self::Index) -> Option<&Self::Piece>;
}

/// Represents a board which can validate candidate moves.
pub trait Validate: Board {
    /// Represents a move which may or may not be legal.
    type Move: Move<Board = Self, Index = Self::Index>;
    /// Represents a move which has been confirmed to be legal.
    type LegalMove: LegalMove<Board = Self, Index = Self::Index>;
    /// The error created when move validation fails.
    type ValidationError: IllegalMoveError<
        Board = Self,
        Index = Self::Index,
        Move = Self::Move,
        LegalMove = Self::LegalMove,
    >;
    /// Validates the given candidate move based on the current state of self.
    fn validate(&self, candidate: Self::Move) -> Result<Self::LegalMove, Self::ValidationError>;
}

/// Represents a board which can process validated moves.
pub trait Process: Validate {
    /// Updates the board state with the given [`LegalMove`] and returns the new state.
    ///
    /// Note that the only valid source for the candidate move is from [`Validate`]'s
    /// `validate` method, and in general you should prefer `validate_and_process` for
    /// updating the board's state with a single [`Move`].
    fn process(&self, candidate: Self::LegalMove) -> Self
    where
        Self: Sized;

    /// First validates the given candidate move, and then either returns an [`IllegalMoveError`]
    /// or uses the resulting [`LegalMove`] to update the board state and returns it.
    fn validate_and_process(&self, candidate: Self::Move) -> Result<Self, Self::ValidationError>
    where
        Self: Sized,
    {
        let legal_move = self.validate(candidate)?;
        Ok(self.process(legal_move))
    }
}
