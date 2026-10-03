//! Native wrapper application. Provider execution, transport and input lifecycle
//! are composed here; session/admission/storage truth remains with the Hub core.
pub mod input;
pub mod launch;
pub mod runtime;
pub mod shell;
pub mod transport;

pub mod entry;
pub mod hooks;
pub mod output;
