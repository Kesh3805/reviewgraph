//! Provider adapters. Each adapter is a thin mapping between [`crate::ProviderRequest`] and a
//! provider wire format.

pub mod anthropic;
pub mod anthropic_wire;
pub(crate) mod http;
pub mod openai;
pub mod openai_wire;
