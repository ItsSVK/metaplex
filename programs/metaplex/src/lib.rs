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

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        ctx.accounts.initialize(args, &ctx.bumps)
    }

    pub fn mint_asset(ctx: Context<MintAsset>, name: String, uri: String) -> Result<()> {
        ctx.accounts.mint_asset(name, uri)
    }

    pub fn stake(ctx: Context<Stake>) -> Result<()> {
        ctx.accounts.stake()
    }

    pub fn claim_rewards(ctx: Context<Position>) -> Result<()> {
        ctx.accounts.claim_rewards()
    }

    pub fn unstake(ctx: Context<Position>) -> Result<()> {
        ctx.accounts.unstake()
    }

    pub fn burn_staked_nft(ctx: Context<Position>) -> Result<()> {
        ctx.accounts.burn_staked_nft()
    }
}
