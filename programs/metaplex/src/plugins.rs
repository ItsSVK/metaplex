use std::ops::Deref;

use anchor_lang::prelude::*;
use mpl_core::accounts::{BaseAssetV1, BaseCollectionV1};
use mpl_core::instructions::{
    AddPluginV1CpiBuilder, BurnV1CpiBuilder, RemovePluginV1CpiBuilder,
    UpdateCollectionPluginV1CpiBuilder, UpdatePluginV1CpiBuilder,
};
use mpl_core::types::{Attribute, Attributes, FreezeDelegate, Key, Plugin, PluginAuthority, PluginType};
use mpl_core::{fetch_asset_plugin, fetch_collection_plugin};

use crate::errors::StakingError;
use crate::state::{Config, STAKED, TOTAL_STAKED};

#[derive(Clone)]
pub struct CoreAsset(BaseAssetV1);

impl AccountDeserialize for CoreAsset {
    fn try_deserialize_unchecked(buf: &mut &[u8]) -> Result<Self> {
        let asset = BaseAssetV1::from_bytes(buf).map_err(|_| error!(StakingError::InvalidAsset))?;
        require!(asset.key == Key::AssetV1, StakingError::InvalidAsset);

        Ok(Self(asset))
    }
}

impl AccountSerialize for CoreAsset {}

impl Owner for CoreAsset {
    fn owner() -> Pubkey {
        mpl_core::ID
    }
}

impl Discriminator for CoreAsset {
    const DISCRIMINATOR: &'static [u8] = &[];
}

#[cfg(feature = "idl-build")]
impl anchor_lang::idl::IdlBuild for CoreAsset {}

impl Deref for CoreAsset {
    type Target = BaseAssetV1;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct CoreCollection(BaseCollectionV1);

impl AccountDeserialize for CoreCollection {
    fn try_deserialize_unchecked(buf: &mut &[u8]) -> Result<Self> {
        let collection = BaseCollectionV1::from_bytes(buf)
            .map_err(|_| error!(StakingError::InvalidCollection))?;
        require!(
            collection.key == Key::CollectionV1,
            StakingError::InvalidCollection
        );

        Ok(Self(collection))
    }
}

impl AccountSerialize for CoreCollection {}

impl Owner for CoreCollection {
    fn owner() -> Pubkey {
        mpl_core::ID
    }
}

impl Discriminator for CoreCollection {
    const DISCRIMINATOR: &'static [u8] = &[];
}

#[cfg(feature = "idl-build")]
impl anchor_lang::idl::IdlBuild for CoreCollection {}

impl Deref for CoreCollection {
    type Target = BaseCollectionV1;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct MplCore;

impl Id for MplCore {
    fn id() -> Pubkey {
        mpl_core::ID
    }
}

pub fn has_freeze_delegate(asset: &AccountInfo) -> bool {
    fetch_asset_plugin::<FreezeDelegate>(asset, PluginType::FreezeDelegate).is_ok()
}

pub fn is_staked(asset: &AccountInfo, authority: &Pubkey) -> bool {
    let frozen_by_program = matches!(
        fetch_asset_plugin::<FreezeDelegate>(asset, PluginType::FreezeDelegate),
        Ok((PluginAuthority::Address { address }, FreezeDelegate { frozen: true }, _))
            if address == *authority
    );

    frozen_by_program
        && asset_attributes(asset)
            .map(|list| value(&list, STAKED) == Some("true"))
            .unwrap_or(false)
}

pub fn asset_attributes(asset: &AccountInfo) -> Option<Vec<Attribute>> {
    fetch_asset_plugin::<Attributes>(asset, PluginType::Attributes)
        .ok()
        .map(|(_, attributes, _)| attributes.attribute_list)
}

pub fn collection_attributes(collection: &AccountInfo) -> Result<Vec<Attribute>> {
    fetch_collection_plugin::<Attributes>(collection, PluginType::Attributes)
        .map(|(_, attributes, _)| attributes.attribute_list)
        .map_err(|_| error!(StakingError::InvalidAttribute))
}

pub fn value<'a>(list: &'a [Attribute], key: &str) -> Option<&'a str> {
    list.iter()
        .find(|attribute| attribute.key == key)
        .map(|attribute| attribute.value.as_str())
}

pub fn read_i64(list: &[Attribute], key: &str) -> Result<i64> {
    value(list, key)
        .and_then(|raw| raw.parse().ok())
        .ok_or_else(|| error!(StakingError::InvalidAttribute))
}

pub fn upsert(list: &mut Vec<Attribute>, key: &str, value: impl ToString) {
    let value = value.to_string();

    match list.iter_mut().find(|attribute| attribute.key == key) {
        Some(attribute) => attribute.value = value,
        None => list.push(Attribute {
            key: key.to_string(),
            value,
        }),
    }
}

pub fn accrued(config: &Config, last_claimed: i64, now: i64) -> Result<(i64, u64)> {
    let elapsed = now
        .checked_sub(last_claimed)
        .ok_or(StakingError::Overflow)?
        .max(0);
    let periods = elapsed / config.period_seconds;
    let amount = u64::try_from(periods)
        .map_err(|_| error!(StakingError::Overflow))?
        .checked_mul(config.rewards_per_period)
        .ok_or(StakingError::Overflow)?;

    Ok((periods, amount))
}

pub fn advance(config: &Config, last_claimed: i64, periods: i64) -> Result<i64> {
    periods
        .checked_mul(config.period_seconds)
        .and_then(|elapsed| last_claimed.checked_add(elapsed))
        .ok_or_else(|| error!(StakingError::Overflow))
}

pub struct CoreCpi<'a, 'b> {
    program: &'b AccountInfo<'a>,
    asset: &'b AccountInfo<'a>,
    collection: &'b AccountInfo<'a>,
    owner: &'b AccountInfo<'a>,
    authority: &'b AccountInfo<'a>,
    system_program: &'b AccountInfo<'a>,
}

impl<'a, 'b> CoreCpi<'a, 'b> {
    pub fn new(infos: &'b [AccountInfo<'a>; 6]) -> Self {
        Self {
            program: &infos[0],
            asset: &infos[1],
            collection: &infos[2],
            owner: &infos[3],
            authority: &infos[4],
            system_program: &infos[5],
        }
    }

    pub fn delegate(&self, plugin: Plugin) -> Result<()> {
        AddPluginV1CpiBuilder::new(self.program)
            .asset(self.asset)
            .collection(Some(self.collection))
            .payer(self.owner)
            .authority(Some(self.owner))
            .system_program(self.system_program)
            .plugin(plugin)
            .init_authority(PluginAuthority::Address {
                address: *self.authority.key,
            })
            .invoke()?;

        Ok(())
    }

    pub fn revoke(&self, plugin_type: PluginType) -> Result<()> {
        RemovePluginV1CpiBuilder::new(self.program)
            .asset(self.asset)
            .collection(Some(self.collection))
            .payer(self.owner)
            .authority(Some(self.owner))
            .system_program(self.system_program)
            .plugin_type(plugin_type)
            .invoke()?;

        Ok(())
    }

    pub fn set_frozen(&self, frozen: bool, seeds: &[&[u8]]) -> Result<()> {
        UpdatePluginV1CpiBuilder::new(self.program)
            .asset(self.asset)
            .collection(Some(self.collection))
            .payer(self.owner)
            .authority(Some(self.authority))
            .system_program(self.system_program)
            .plugin(Plugin::FreezeDelegate(FreezeDelegate { frozen }))
            .invoke_signed(&[seeds])?;

        Ok(())
    }

    pub fn write_attributes(
        &self,
        attribute_list: Vec<Attribute>,
        exists: bool,
        seeds: &[&[u8]],
    ) -> Result<()> {
        let plugin = Plugin::Attributes(Attributes { attribute_list });

        if exists {
            UpdatePluginV1CpiBuilder::new(self.program)
                .asset(self.asset)
                .collection(Some(self.collection))
                .payer(self.owner)
                .authority(Some(self.authority))
                .system_program(self.system_program)
                .plugin(plugin)
                .invoke_signed(&[seeds])?;
        } else {
            AddPluginV1CpiBuilder::new(self.program)
                .asset(self.asset)
                .collection(Some(self.collection))
                .payer(self.owner)
                .authority(Some(self.authority))
                .system_program(self.system_program)
                .plugin(plugin)
                .invoke_signed(&[seeds])?;
        }

        Ok(())
    }

    pub fn adjust_total_staked(&self, delta: i64, seeds: &[&[u8]]) -> Result<()> {
        let mut attribute_list = collection_attributes(self.collection)?;
        let total = read_i64(&attribute_list, TOTAL_STAKED)?
            .checked_add(delta)
            .filter(|total| *total >= 0)
            .ok_or(StakingError::Overflow)?;
        upsert(&mut attribute_list, TOTAL_STAKED, total);

        UpdateCollectionPluginV1CpiBuilder::new(self.program)
            .collection(self.collection)
            .payer(self.owner)
            .authority(Some(self.authority))
            .system_program(self.system_program)
            .plugin(Plugin::Attributes(Attributes { attribute_list }))
            .invoke_signed(&[seeds])?;

        Ok(())
    }

    pub fn burn(&self, seeds: &[&[u8]]) -> Result<()> {
        BurnV1CpiBuilder::new(self.program)
            .asset(self.asset)
            .collection(Some(self.collection))
            .payer(self.owner)
            .authority(Some(self.authority))
            .system_program(Some(self.system_program))
            .invoke_signed(&[seeds])?;

        Ok(())
    }
}
