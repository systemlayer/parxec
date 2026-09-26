/// How a command ended without an operational error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
  Completed,
  Cancelled,
}
