#![feature(result_flattening)]

mod server;
#[cfg(test)]
mod tests;

pub use server::Backend;
