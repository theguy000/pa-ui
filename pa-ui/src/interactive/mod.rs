//! One function per control in the window.
//!
//! main.rs builds the window and its state; these register the callbacks. Each
//! takes exactly the handles its callbacks need, so a callback cannot reach into
//! something it was not given - the parameter list is the dependency list.

#[allow(unused_imports)]
use crate::*;

pub(crate) mod attachments;
pub(crate) mod canvas;
pub(crate) mod menu;
pub(crate) mod queue;
pub(crate) mod rail;
pub(crate) mod rows;
pub(crate) mod send;
pub(crate) mod slash;
