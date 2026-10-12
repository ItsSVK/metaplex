use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token};
use mpl_core::instructions::CreateCollectionV2CpiBuilder;
use mpl_core::types::{Attribute, Attributes, Plugin, PluginAuthority, PluginAuthorityPair};

use crate::errors::StakingError;
use crate::plugins::MplCore;
use crate::state::{
    Config, AUTHORITY_SEED, CONFIG_SEED, REWARDS_DECIMALS, REWARDS_SEED, TOTAL_STAKED,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitializeArgs {
    pub name: String,
    pub uri: String,
    pub rewards_per_period: u64,
    pub period_seconds: i64,
    pub min_stake_seconds: i64,
    pub burn_bonus: u64,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(mut)]
    pub collection: Signer<'info>,

    #[account(seeds = [AUTHORITY_SEED, collection.key().as_ref()], bump)]
    pub update_authority: SystemAccount<'info>,

    #[account(
        init,
        payer = admin,
        space = 8 + Config::INIT_SPACE,
        seeds = [CONFIG_SEED, collection.key().as_ref()],
        bump
    )]
    pub config: Account<'info, Config>,

    #[account(
        init,
        payer = admin,
        seeds = [REWARDS_SEED, config.key().as_ref()],
        bump,
        mint::decimals = REWARDS_DECIMALS,
        mint::authority = config
    )]
    pub rewards_mint: Account<'info, Mint>,

    pub core_program: Program<'info, MplCore>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'info> Initialize<'info> {
    pub fn initialize(&mut self, args: InitializeArgs, bumps: &InitializeBumps) -> Result<()> {
        require!(
            args.period_seconds > 0 && args.min_stake_seconds >= 0,
            StakingError::InvalidPeriod
        );
        require!(args.rewards_per_period > 0, StakingError::InvalidRate);

        self.config.set_inner(Config {
            admin: self.admin.key(),
            collection: self.collection.key(),
            rewards_per_period: args.rewards_per_period,
            period_seconds: args.period_seconds,
            min_stake_seconds: args.min_stake_seconds,
            burn_bonus: args.burn_bonus,
            config_bump: bumps.config,
            authority_bump: bumps.update_authority,
            rewards_bump: bumps.rewards_mint,
        });

        CreateCollectionV2CpiBuilder::new(&self.core_program.to_account_info())
            .collection(&self.collection.to_account_info())
            .update_authority(Some(&self.update_authority.to_account_info()))
            .payer(&self.admin.to_account_info())
            .system_program(&self.system_program.to_account_info())
            .name(args.name)
            .uri(args.uri)
            .plugins(vec![PluginAuthorityPair {
                plugin: Plugin::Attributes(Attributes {
                    attribute_list: vec![Attribute {
                        key: TOTAL_STAKED.to_string(),
                        value: "0".to_string(),
                    }],
                }),
                authority: Some(PluginAuthority::UpdateAuthority),
            }])
            .invoke()?;

        Ok(())
    }
}
