//! Stack-usage guard. Recursive parsing, evaluation and data traversal check how much
//! stack the current render has used and raise `LimitError` before the thread's stack
//! could overflow, whatever the frame sizes of the build.

use crate::error::{fail, R, LIMIT_ERROR};
use std::cell::Cell;

/// Stack bytes a render may use. Spawned Rust threads get 2 MiB by default.
pub const STACK_BUDGET: usize = 1 << 20;

/// Message of the LimitError raised when the stack budget runs out.
pub const STACK_LIMIT_MESSAGE: &str = "nesting too deep (stack limit)";

thread_local! {
    static BASE: Cell<usize> = const { Cell::new(0) };
    static BUDGET: Cell<usize> = const { Cell::new(STACK_BUDGET) };
}

#[inline(never)]
fn here() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

/// Marks the start of a render on this thread (nested renders keep the outer base).
pub struct Scope {
    owner: bool,
}

pub fn enter() -> Scope {
    enter_with(STACK_BUDGET)
}

/// Like `enter`, for a thread known to have `budget` bytes of stack to spare.
pub fn enter_with(budget: usize) -> Scope {
    BUDGET.with(|b| b.set(budget));
    let owner = BASE.with(|b| {
        if b.get() == 0 {
            b.set(here());
            true
        } else {
            false
        }
    });
    Scope { owner }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.owner {
            BASE.with(|b| b.set(0));
        }
    }
}

/// True when the render has used most of its stack budget.
pub fn exhausted() -> bool {
    let base = BASE.with(|b| b.get());
    base != 0 && base.saturating_sub(here()) > BUDGET.with(|b| b.get())
}

/// Raises LimitError when the stack budget is used up.
pub fn check() -> R<()> {
    if exhausted() {
        return fail(LIMIT_ERROR, STACK_LIMIT_MESSAGE);
    }
    Ok(())
}

