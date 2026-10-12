use anchor_lang::prelude::*;
use mpl_core::types::{BurnDelegate, FreezeDelegate, Plugin, UpdateAuthority};

use crate::errors::StakingError;
use crate::plugins::{
    asset_attributes, has_freeze_delegate, upsert, CoreAsset, CoreCollection, CoreCpi, MplCore,
};
use crate::state::{Config, AUTHORITY_SEED, CONFIG_SEED, LAST_CLAIMED, STAKED, STAKED_AT};

#[derive(Accounts)]
pub struct Stake<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        constraint = asset.owner == owner.key() @ StakingError::NotOwner,
        constraint = asset.update_authority == UpdateAuthority::Collection(collection.key())
            @ StakingError::WrongCollection
    )]
    pub asset: Box<Account<'info, CoreAsset>>,

    #[account(mut)]
    pub collection: Box<Account<'info, CoreCollection>>,

    #[account(
        seeds = [AUTHORITY_SEED, collection.key().as_ref()],
        bump = config.authority_bump
    )]
    pub update_authority: SystemAccount<'info>,

    #[account(
        has_one = collection,
        seeds = [CONFIG_SEED, collection.key().as_ref()],
        bump = config.config_bump
    )]
    pub config: Box<Account<'info, Config>>,

    pub core_program: Program<'info, MplCore>,
    pub system_program: Program<'info, System>,
}

impl<'info> Stake<'info> {
    pub fn stake(&mut self) -> Result<()> {
        let infos = [
            self.core_program.to_account_info(),
            self.asset.to_account_info(),
            self.collection.to_account_info(),
            self.owner.to_account_info(),
            self.update_authority.to_account_info(),
            self.system_program.to_account_info(),
        ];

        require!(!has_freeze_delegate(&infos[1]), StakingError::AlreadyStaked);

        let existing = asset_attributes(&infos[1]);
        let now = Clock::get()?.unix_timestamp;

        let mut attributes = existing.clone().unwrap_or_default();
        upsert(&mut attributes, STAKED, "true");
        upsert(&mut attributes, STAKED_AT, now);
        upsert(&mut attributes, LAST_CLAIMED, now);

        let seeds = self.config.authority_seeds();
        let core = CoreCpi::new(&infos);

        core.delegate(Plugin::FreezeDelegate(FreezeDelegate { frozen: true }))?;
        core.delegate(Plugin::BurnDelegate(BurnDelegate {}))?;
        core.write_attributes(attributes, existing.is_some(), &seeds)?;
        core.adjust_total_staked(1, &seeds)
    }
}
