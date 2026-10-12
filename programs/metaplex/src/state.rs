use anchor_lang::prelude::*;

pub const CONFIG_SEED: &[u8] = b"config";
pub const AUTHORITY_SEED: &[u8] = b"update_authority";
pub const REWARDS_SEED: &[u8] = b"rewards";

pub const REWARDS_DECIMALS: u8 = 6;

pub const TOTAL_STAKED: &str = "total_staked";
pub const STAKED: &str = "staked";
pub const STAKED_AT: &str = "staked_at";
pub const LAST_CLAIMED: &str = "last_claimed";

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey,
    pub collection: Pubkey,
    pub rewards_per_period: u64,
    pub period_seconds: i64,
    pub min_stake_seconds: i64,
    pub burn_bonus: u64,
    pub config_bump: u8,
    pub authority_bump: u8,
    pub rewards_bump: u8,
}

impl Config {
    pub fn config_seeds(&self) -> [&[u8]; 3] {
        [
            CONFIG_SEED,
            self.collection.as_ref(),
            core::slice::from_ref(&self.config_bump),
        ]
    }

    pub fn authority_seeds(&self) -> [&[u8]; 3] {
        [
            AUTHORITY_SEED,
            self.collection.as_ref(),
            core::slice::from_ref(&self.authority_bump),
        ]
    }
}
