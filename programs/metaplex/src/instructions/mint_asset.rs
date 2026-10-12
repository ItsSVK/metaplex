use anchor_lang::prelude::*;
use mpl_core::instructions::CreateV2CpiBuilder;

use crate::plugins::{CoreCollection, MplCore};
use crate::state::{Config, AUTHORITY_SEED, CONFIG_SEED};

#[derive(Accounts)]
pub struct MintAsset<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    pub owner: SystemAccount<'info>,

    #[account(mut)]
    pub asset: Signer<'info>,

    #[account(mut)]
    pub collection: Box<Account<'info, CoreCollection>>,

    #[account(
        seeds = [AUTHORITY_SEED, collection.key().as_ref()],
        bump = config.authority_bump
    )]
    pub update_authority: SystemAccount<'info>,

    #[account(
        has_one = admin,
        has_one = collection,
        seeds = [CONFIG_SEED, collection.key().as_ref()],
        bump = config.config_bump
    )]
    pub config: Box<Account<'info, Config>>,

    pub core_program: Program<'info, MplCore>,
    pub system_program: Program<'info, System>,
}

impl<'info> MintAsset<'info> {
    pub fn mint_asset(&mut self, name: String, uri: String) -> Result<()> {
        let seeds = self.config.authority_seeds();

        CreateV2CpiBuilder::new(&self.core_program.to_account_info())
            .asset(&self.asset.to_account_info())
            .collection(Some(&self.collection.to_account_info()))
            .authority(Some(&self.update_authority.to_account_info()))
            .payer(&self.admin.to_account_info())
            .owner(Some(&self.owner.to_account_info()))
            .system_program(&self.system_program.to_account_info())
            .name(name)
            .uri(uri)
            .invoke_signed(&[&seeds])?;

        Ok(())
    }
}
