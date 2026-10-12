pub mod errors;
pub mod instructions;
pub mod plugins;
pub mod state;

use anchor_lang::prelude::*;

pub use errors::*;
pub use instructions::*;
pub use plugins::*;
pub use state::*;

declare_id!("AH3hGfZeBmrgydfcw9NdWh5Lv6aHau4D2YhM6fiv7tmD");

#[program]
pub mod metaplex {
    use super::*;
}
