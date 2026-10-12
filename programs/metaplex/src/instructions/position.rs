use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{mint_to, Mint, MintTo, Token, TokenAccount};
use mpl_core::types::{Attribute, PluginType, UpdateAuthority};

use crate::errors::StakingError;
use crate::plugins::{
    accrued, advance, asset_attributes, is_staked, read_i64, upsert, CoreAsset, CoreCollection,
    CoreCpi, MplCore,
};
use crate::state::{
    Config, AUTHORITY_SEED, CONFIG_SEED, LAST_CLAIMED, REWARDS_SEED, STAKED, STAKED_AT,
};

#[derive(Accounts)]
pub struct Position<'info> {
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

    #[account(
        mut,
        seeds = [REWARDS_SEED, config.key().as_ref()],
        bump = config.rewards_bump
    )]
    pub rewards_mint: Box<Account<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = owner,
        associated_token::mint = rewards_mint,
        associated_token::authority = owner
    )]
    pub owner_rewards: Box<Account<'info, TokenAccount>>,

    pub core_program: Program<'info, MplCore>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> Position<'info> {
    pub fn claim_rewards(&mut self) -> Result<()> {
        let (mut attributes, _, last_claimed) = self.staked_position()?;
        let (periods, amount) = accrued(&self.config, last_claimed, Clock::get()?.unix_timestamp)?;
        require!(amount > 0, StakingError::NothingToClaim);

        self.mint_rewards(amount)?;
        upsert(
            &mut attributes,
            LAST_CLAIMED,
            advance(&self.config, last_claimed, periods)?,
        );

        let infos = self.core_infos();
        let seeds = self.config.authority_seeds();

        CoreCpi::new(&infos).write_attributes(attributes, true, &seeds)
    }

    pub fn unstake(&mut self) -> Result<()> {
        let (mut attributes, staked_at, last_claimed) = self.staked_position()?;
        let now = Clock::get()?.unix_timestamp;

        require!(
            now.checked_sub(staked_at).ok_or(StakingError::Overflow)?
                >= self.config.min_stake_seconds,
            StakingError::StakeLocked
        );

        let (periods, amount) = accrued(&self.config, last_claimed, now)?;
        if amount > 0 {
            self.mint_rewards(amount)?;
        }

        upsert(&mut attributes, STAKED, "false");
        upsert(
            &mut attributes,
            LAST_CLAIMED,
            advance(&self.config, last_claimed, periods)?,
        );

        let infos = self.core_infos();
        let seeds = self.config.authority_seeds();
        let core = CoreCpi::new(&infos);

        core.set_frozen(false, &seeds)?;
        core.revoke(PluginType::FreezeDelegate)?;
        core.revoke(PluginType::BurnDelegate)?;
        core.write_attributes(attributes, true, &seeds)?;
        core.adjust_total_staked(-1, &seeds)
    }

    pub fn burn_staked_nft(&mut self) -> Result<()> {
        let (_, _, last_claimed) = self.staked_position()?;
        let (_, amount) = accrued(&self.config, last_claimed, Clock::get()?.unix_timestamp)?;
        let reward = amount
            .checked_add(self.config.burn_bonus)
            .ok_or(StakingError::Overflow)?;

        if reward > 0 {
            self.mint_rewards(reward)?;
        }

        let infos = self.core_infos();
        let seeds = self.config.authority_seeds();
        let core = CoreCpi::new(&infos);

        core.set_frozen(false, &seeds)?;
        core.burn(&seeds)?;
        core.adjust_total_staked(-1, &seeds)
    }

    fn staked_position(&self) -> Result<(Vec<Attribute>, i64, i64)> {
        let asset = self.asset.to_account_info();
        require!(
            is_staked(&asset, &self.update_authority.key()),
            StakingError::NotStaked
        );

        let attributes =
            asset_attributes(&asset).ok_or_else(|| error!(StakingError::InvalidAttribute))?;
        let staked_at = read_i64(&attributes, STAKED_AT)?;
        let last_claimed = read_i64(&attributes, LAST_CLAIMED)?;

        Ok((attributes, staked_at, last_claimed))
    }

    fn core_infos(&self) -> [AccountInfo<'info>; 6] {
        [
            self.core_program.to_account_info(),
            self.asset.to_account_info(),
            self.collection.to_account_info(),
            self.owner.to_account_info(),
            self.update_authority.to_account_info(),
            self.system_program.to_account_info(),
        ]
    }

    fn mint_rewards(&self, amount: u64) -> Result<()> {
        let seeds = self.config.config_seeds();

        mint_to(
            CpiContext::new_with_signer(
                self.token_program.key(),
                MintTo {
                    mint: self.rewards_mint.to_account_info(),
                    to: self.owner_rewards.to_account_info(),
                    authority: self.config.to_account_info(),
                },
                &[&seeds],
            ),
            amount,
        )
    }
}
