//! LiteSVM integration tests for the Core NFT staking program.
//!
//! Run `cargo build-sbf` first so `target/deploy/metaplex.so` is up to date
//! (the `anchor test` script in Anchor.toml does this for you).
//! `tests/fixtures/mpl_core.so` is a dump of the public mpl-core program:
//! `solana program dump -u mainnet-beta CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d tests/fixtures/mpl_core.so`

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{system_program, InstructionData, ToAccountMetas};
use anchor_spl::associated_token::{get_associated_token_address, ID as ASSOCIATED_TOKEN_ID};
use anchor_spl::token::{Mint, TokenAccount, ID as TOKEN_ID};
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use litesvm::LiteSVM;
use metaplex::{
    accounts, instruction, Config, InitializeArgs, StakingError, AUTHORITY_SEED, CONFIG_SEED,
    LAST_CLAIMED, REWARDS_SEED, STAKED, STAKED_AT, TOTAL_STAKED,
};
use mpl_core::accounts::BaseAssetV1;
use mpl_core::types::{Attributes, FreezeDelegate, PluginType, UpdateAuthority};
use mpl_core::{fetch_asset_plugin, fetch_collection_plugin};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::{InstructionError, Transaction, TransactionError};

const PROGRAM_SO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/deploy/metaplex.so"
);
const MPL_CORE_SO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mpl_core.so");

const START_TIME: i64 = 1_700_000_000;
const REWARD: u64 = 100;
const PERIOD: i64 = 60;
const MIN_STAKE: i64 = 300;
const BURN_BONUS: u64 = 1_000;

type TxResult = std::result::Result<TransactionMetadata, FailedTransactionMetadata>;

/// One staking collection (its own admin, config and rewards mint).
struct Pool {
    admin: Keypair,
    collection: Keypair,
}

impl Pool {
    fn authority(&self) -> Pubkey {
        Pubkey::find_program_address(
            &[AUTHORITY_SEED, self.collection.pubkey().as_ref()],
            &metaplex::ID,
        )
        .0
    }

    fn config(&self) -> Pubkey {
        Pubkey::find_program_address(
            &[CONFIG_SEED, self.collection.pubkey().as_ref()],
            &metaplex::ID,
        )
        .0
    }

    fn rewards_mint(&self) -> Pubkey {
        Pubkey::find_program_address(&[REWARDS_SEED, self.config().as_ref()], &metaplex::ID).0
    }
}

struct Env {
    svm: LiteSVM,
    pool: Pool,
    owner: Keypair,
}

impl Env {
    /// SVM with both programs loaded, funded wallets, and an initialised pool.
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(metaplex::ID, PROGRAM_SO)
            .expect("run `cargo build-sbf` before the tests");
        svm.add_program_from_file(mpl_core::ID, MPL_CORE_SO)
            .expect("mpl_core.so fixture is missing");
        set_time(&mut svm, START_TIME);

        let owner = funded(&mut svm);
        let mut env = Self {
            svm,
            pool: Pool {
                admin: Keypair::new(),
                collection: Keypair::new(),
            },
            owner,
        };
        env.svm
            .airdrop(&env.pool.admin.pubkey(), 100_000_000_000)
            .unwrap();
        let pool = Pool {
            admin: env.pool.admin.insecure_clone(),
            collection: env.pool.collection.insecure_clone(),
        };
        env.initialize(&pool, REWARD, PERIOD).unwrap();
        env
    }

    fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
        let tx = Transaction::new(
            signers,
            Message::new(ixs, Some(&signers[0].pubkey())),
            self.svm.latest_blockhash(),
        );
        let res = self.svm.send_transaction(tx);
        // Identical retries would otherwise be rejected as AlreadyProcessed.
        self.svm.expire_blockhash();
        res
    }

    fn initialize(&mut self, pool: &Pool, rate: u64, period: i64) -> TxResult {
        let ix = Instruction {
            program_id: metaplex::ID,
            accounts: accounts::Initialize {
                admin: pool.admin.pubkey(),
                collection: pool.collection.pubkey(),
                update_authority: pool.authority(),
                config: pool.config(),
                rewards_mint: pool.rewards_mint(),
                core_program: mpl_core::ID,
                token_program: TOKEN_ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::Initialize {
                args: InitializeArgs {
                    name: "Test Collection".into(),
                    uri: "https://example.invalid/collection.json".into(),
                    rewards_per_period: rate,
                    period_seconds: period,
                    min_stake_seconds: MIN_STAKE,
                    burn_bonus: BURN_BONUS,
                },
            }
            .data(),
        };
        self.send(&[ix], &[&pool.admin, &pool.collection])
    }

    fn mint_as(&mut self, pool: &Pool, signer: &Keypair, owner: &Pubkey) -> (Keypair, TxResult) {
        let asset = Keypair::new();
        let ix = Instruction {
            program_id: metaplex::ID,
            accounts: accounts::MintAsset {
                admin: signer.pubkey(),
                owner: *owner,
                asset: asset.pubkey(),
                collection: pool.collection.pubkey(),
                update_authority: pool.authority(),
                config: pool.config(),
                core_program: mpl_core::ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::MintAsset {
                name: "Test NFT".into(),
                uri: "https://example.invalid/nft.json".into(),
            }
            .data(),
        };
        let res = self.send(&[ix], &[signer, &asset]);
        (asset, res)
    }

    /// Mints an asset from the default pool to the default owner.
    fn mint(&mut self) -> Keypair {
        let admin = self.pool.admin.insecure_clone();
        let owner = self.owner.pubkey();
        let pool = Pool {
            admin: admin.insecure_clone(),
            collection: self.pool.collection.insecure_clone(),
        };
        let (asset, res) = self.mint_as(&pool, &admin, &owner);
        res.expect("mint_asset failed");
        asset
    }

    fn position_accounts(&self, signer: &Pubkey, asset: &Pubkey) -> Vec<AccountMeta> {
        accounts::Position {
            owner: *signer,
            asset: *asset,
            collection: self.pool.collection.pubkey(),
            update_authority: self.pool.authority(),
            config: self.pool.config(),
            rewards_mint: self.pool.rewards_mint(),
            owner_rewards: get_associated_token_address(signer, &self.pool.rewards_mint()),
            core_program: mpl_core::ID,
            token_program: TOKEN_ID,
            associated_token_program: ASSOCIATED_TOKEN_ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None)
    }

    fn stake_as(&mut self, signer: &Keypair, asset: &Keypair) -> TxResult {
        let ix = Instruction {
            program_id: metaplex::ID,
            accounts: accounts::Stake {
                owner: signer.pubkey(),
                asset: asset.pubkey(),
                collection: self.pool.collection.pubkey(),
                update_authority: self.pool.authority(),
                config: self.pool.config(),
                core_program: mpl_core::ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::Stake {}.data(),
        };
        self.send(&[ix], &[signer])
    }

    fn stake(&mut self, asset: &Keypair) -> TxResult {
        let owner = self.owner.insecure_clone();
        self.stake_as(&owner, asset)
    }

    fn position_ix(&mut self, asset: &Keypair, data: Vec<u8>) -> TxResult {
        let owner = self.owner.insecure_clone();
        let ix = Instruction {
            program_id: metaplex::ID,
            accounts: self.position_accounts(&owner.pubkey(), &asset.pubkey()),
            data,
        };
        self.send(&[ix], &[&owner])
    }

    fn claim(&mut self, asset: &Keypair) -> TxResult {
        self.position_ix(asset, instruction::ClaimRewards {}.data())
    }

    fn unstake(&mut self, asset: &Keypair) -> TxResult {
        self.position_ix(asset, instruction::Unstake {}.data())
    }

    fn burn(&mut self, asset: &Keypair) -> TxResult {
        self.position_ix(asset, instruction::BurnStakedNft {}.data())
    }

    fn advance(&mut self, seconds: i64) {
        let now = self.svm.get_sysvar::<Clock>().unix_timestamp;
        set_time(&mut self.svm, now + seconds);
    }

    // ---- state readers (go through mpl-core directly, not the program's helpers) ----

    fn config_state(&self) -> Config {
        let account = self.svm.get_account(&self.pool.config()).expect("config");
        Config::try_deserialize(&mut account.data.as_slice()).unwrap()
    }

    fn rewards_balance(&self) -> u64 {
        let ata = get_associated_token_address(&self.owner.pubkey(), &self.pool.rewards_mint());
        match self.svm.get_account(&ata) {
            Some(account) => {
                TokenAccount::try_deserialize(&mut account.data.as_slice())
                    .unwrap()
                    .amount
            }
            None => 0,
        }
    }

    fn rewards_supply(&self) -> u64 {
        let account = self
            .svm
            .get_account(&self.pool.rewards_mint())
            .expect("rewards mint");
        Mint::try_deserialize(&mut account.data.as_slice())
            .unwrap()
            .supply
    }

    fn asset_owner(&self, asset: &Pubkey) -> (Pubkey, UpdateAuthority) {
        let account = self.svm.get_account(asset).expect("asset");
        let base = BaseAssetV1::from_bytes(&account.data).unwrap();
        (base.owner, base.update_authority)
    }

    fn asset_attr(&self, asset: &Pubkey, key: &str) -> Option<String> {
        with_info(&self.svm, asset, |info| {
            fetch_asset_plugin::<Attributes>(info, PluginType::Attributes)
                .ok()?
                .1
                .attribute_list
                .into_iter()
                .find(|a| a.key == key)
                .map(|a| a.value)
        })
    }

    fn frozen(&self, asset: &Pubkey) -> Option<bool> {
        with_info(&self.svm, asset, |info| {
            fetch_asset_plugin::<FreezeDelegate>(info, PluginType::FreezeDelegate)
                .ok()
                .map(|(_, plugin, _)| plugin.frozen)
        })
    }

    fn has_plugin(&self, asset: &Pubkey, plugin: PluginType) -> bool {
        with_info(&self.svm, asset, |info| match plugin {
            PluginType::BurnDelegate => {
                fetch_asset_plugin::<mpl_core::types::BurnDelegate>(info, plugin).is_ok()
            }
            PluginType::FreezeDelegate => {
                fetch_asset_plugin::<FreezeDelegate>(info, plugin).is_ok()
            }
            _ => unreachable!("extend has_plugin for {plugin:?}"),
        })
    }

    fn total_staked(&self, pool: &Pool) -> i64 {
        with_info(&self.svm, &pool.collection.pubkey(), |info| {
            fetch_collection_plugin::<Attributes>(info, PluginType::Attributes)
                .expect("collection attributes")
                .1
                .attribute_list
                .into_iter()
                .find(|a| a.key == TOTAL_STAKED)
                .expect("total_staked attribute")
                .value
                .parse()
                .unwrap()
        })
    }
}

fn funded(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 100_000_000_000).unwrap();
    kp
}

fn set_time(svm: &mut LiteSVM, unix_timestamp: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp = unix_timestamp;
    svm.set_sysvar(&clock);
}

fn with_info<R>(svm: &LiteSVM, key: &Pubkey, f: impl FnOnce(&AccountInfo) -> R) -> R {
    let mut account = svm.get_account(key).expect("account should exist");
    let mut lamports = account.lamports;
    let info = AccountInfo::new(
        key,
        false,
        false,
        &mut lamports,
        &mut account.data,
        &account.owner,
        account.executable,
    );
    f(&info)
}

#[track_caller]
fn assert_staking_error(res: TxResult, expected: StakingError) {
    let failed = res.expect_err("transaction should have failed");
    let code = ERROR_CODE_OFFSET + expected as u32;
    match &failed.err {
        TransactionError::InstructionError(_, InstructionError::Custom(actual)) => assert_eq!(
            *actual, code,
            "expected {expected:?} ({code}), logs: {:#?}",
            failed.meta.logs
        ),
        other => panic!(
            "expected {expected:?} ({code}), got {other:?}, logs: {:#?}",
            failed.meta.logs
        ),
    }
}

/// Anchor constraint violations (e.g. `has_one`) use framework error codes.
#[track_caller]
fn assert_anchor_error(res: TxResult, expected: anchor_lang::error::ErrorCode) {
    let failed = res.expect_err("transaction should have failed");
    let code = expected as u32;
    match &failed.err {
        TransactionError::InstructionError(_, InstructionError::Custom(actual)) => assert_eq!(
            *actual, code,
            "expected {expected:?} ({code}), logs: {:#?}",
            failed.meta.logs
        ),
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

// ---------------------------------------------------------------- initialize

#[test]
fn initialize_creates_config_collection_and_rewards_mint() {
    let env = Env::new();
    let config = env.config_state();

    assert_eq!(config.admin, env.pool.admin.pubkey());
    assert_eq!(config.collection, env.pool.collection.pubkey());
    assert_eq!(config.rewards_per_period, REWARD);
    assert_eq!(config.period_seconds, PERIOD);
    assert_eq!(config.min_stake_seconds, MIN_STAKE);
    assert_eq!(config.burn_bonus, BURN_BONUS);

    let mint = env.svm.get_account(&env.pool.rewards_mint()).unwrap();
    let mint = Mint::try_deserialize(&mut mint.data.as_slice()).unwrap();
    assert_eq!(mint.decimals, metaplex::REWARDS_DECIMALS);
    assert_eq!(
        Option::<Pubkey>::from(mint.mint_authority),
        Some(env.pool.config())
    );
    assert_eq!(mint.supply, 0);

    // The collection starts with a zero counter, controlled by the PDA authority.
    assert_eq!(env.total_staked(&env.pool), 0);
}

#[test]
fn initialize_rejects_zero_reward_rate() {
    let mut env = Env::new();
    let pool = Pool {
        admin: funded(&mut env.svm),
        collection: Keypair::new(),
    };
    assert_staking_error(env.initialize(&pool, 0, PERIOD), StakingError::InvalidRate);
}

#[test]
fn initialize_rejects_non_positive_period() {
    let mut env = Env::new();
    for period in [0, -1] {
        let pool = Pool {
            admin: funded(&mut env.svm),
            collection: Keypair::new(),
        };
        assert_staking_error(
            env.initialize(&pool, REWARD, period),
            StakingError::InvalidPeriod,
        );
    }
}

// ---------------------------------------------------------------- mint_asset

#[test]
fn admin_can_mint_asset_into_collection() {
    let mut env = Env::new();
    let asset = env.mint();

    let (owner, authority) = env.asset_owner(&asset.pubkey());
    assert_eq!(owner, env.owner.pubkey());
    assert_eq!(
        authority,
        UpdateAuthority::Collection(env.pool.collection.pubkey())
    );
}

#[test]
fn non_admin_cannot_mint_asset() {
    let mut env = Env::new();
    let attacker = funded(&mut env.svm);
    let owner = env.owner.pubkey();
    let pool = Pool {
        admin: env.pool.admin.insecure_clone(),
        collection: env.pool.collection.insecure_clone(),
    };

    let (_, res) = env.mint_as(&pool, &attacker, &owner);
    assert_anchor_error(res, anchor_lang::error::ErrorCode::ConstraintHasOne);
}

// --------------------------------------------------------------------- stake

#[test]
fn stake_freezes_asset_and_records_position() {
    let mut env = Env::new();
    let asset = env.mint();
    env.stake(&asset).expect("stake failed");

    let key = asset.pubkey();
    assert_eq!(env.frozen(&key), Some(true));
    assert!(env.has_plugin(&key, PluginType::BurnDelegate));
    assert_eq!(env.asset_attr(&key, STAKED).as_deref(), Some("true"));
    assert_eq!(
        env.asset_attr(&key, STAKED_AT),
        Some(START_TIME.to_string())
    );
    assert_eq!(
        env.asset_attr(&key, LAST_CLAIMED),
        Some(START_TIME.to_string())
    );
    assert_eq!(env.total_staked(&env.pool), 1);
}

#[test]
fn stake_twice_is_rejected() {
    let mut env = Env::new();
    let asset = env.mint();
    env.stake(&asset).unwrap();

    assert_staking_error(env.stake(&asset), StakingError::AlreadyStaked);
    assert_eq!(env.total_staked(&env.pool), 1);
}

#[test]
fn only_the_owner_can_stake() {
    let mut env = Env::new();
    let asset = env.mint();
    let stranger = funded(&mut env.svm);

    assert_staking_error(env.stake_as(&stranger, &asset), StakingError::NotOwner);
    assert_eq!(env.frozen(&asset.pubkey()), None);
    assert_eq!(env.total_staked(&env.pool), 0);
}

#[test]
fn asset_from_another_collection_cannot_be_staked() {
    let mut env = Env::new();

    // A second, independent pool with its own asset.
    let other = Pool {
        admin: funded(&mut env.svm),
        collection: Keypair::new(),
    };
    env.initialize(&other, REWARD, PERIOD).unwrap();
    let owner = env.owner.pubkey();
    let (foreign_asset, res) = env.mint_as(&other, &other.admin.insecure_clone(), &owner);
    res.unwrap();

    // Present the foreign asset against the first pool's config and collection.
    assert_staking_error(env.stake(&foreign_asset), StakingError::WrongCollection);
}

// ------------------------------------------------------------- claim_rewards

#[test]
fn claim_before_a_full_period_has_nothing_to_claim() {
    let mut env = Env::new();
    let asset = env.mint();
    env.stake(&asset).unwrap();

    env.advance(PERIOD - 1);
    assert_staking_error(env.claim(&asset), StakingError::NothingToClaim);
    assert_eq!(env.rewards_balance(), 0);
}

#[test]
fn claim_pays_whole_periods_and_keeps_the_remainder() {
    let mut env = Env::new();
    let asset = env.mint();
    env.stake(&asset).unwrap();

    // 2.5 periods elapsed -> only 2 are paid, the half period carries over.
    env.advance(PERIOD * 5 / 2);
    env.claim(&asset).expect("claim failed");

    assert_eq!(env.rewards_balance(), 2 * REWARD);
    assert_eq!(env.rewards_supply(), 2 * REWARD);
    assert_eq!(
        env.asset_attr(&asset.pubkey(), LAST_CLAIMED),
        Some((START_TIME + 2 * PERIOD).to_string())
    );

    // Immediately claiming again pays nothing.
    assert_staking_error(env.claim(&asset), StakingError::NothingToClaim);

    // The carried-over half period completes a third period.
    env.advance(PERIOD / 2);
    env.claim(&asset).unwrap();
    assert_eq!(env.rewards_balance(), 3 * REWARD);
}

#[test]
fn claim_requires_a_staked_asset() {
    let mut env = Env::new();
    let asset = env.mint();
    env.advance(PERIOD * 10);

    assert_staking_error(env.claim(&asset), StakingError::NotStaked);
}

// ------------------------------------------------------------------- unstake

#[test]
fn unstake_is_locked_during_the_minimum_period() {
    let mut env = Env::new();
    let asset = env.mint();
    env.stake(&asset).unwrap();

    env.advance(MIN_STAKE - 1);
    assert_staking_error(env.unstake(&asset), StakingError::StakeLocked);
    assert_eq!(env.frozen(&asset.pubkey()), Some(true));
    assert_eq!(env.total_staked(&env.pool), 1);
}

#[test]
fn unstake_pays_rewards_and_releases_the_asset() {
    let mut env = Env::new();
    let asset = env.mint();
    env.stake(&asset).unwrap();

    env.advance(MIN_STAKE); // 5 full periods
    env.unstake(&asset).expect("unstake failed");

    let key = asset.pubkey();
    assert_eq!(env.rewards_balance(), 5 * REWARD);
    assert_eq!(env.asset_attr(&key, STAKED).as_deref(), Some("false"));
    assert!(!env.has_plugin(&key, PluginType::FreezeDelegate));
    assert!(!env.has_plugin(&key, PluginType::BurnDelegate));
    assert_eq!(env.total_staked(&env.pool), 0);

    // Still owned by the staker the whole time.
    assert_eq!(env.asset_owner(&key).0, env.owner.pubkey());
}

#[test]
fn unstaked_asset_can_be_staked_again() {
    let mut env = Env::new();
    let asset = env.mint();
    env.stake(&asset).unwrap();
    env.advance(MIN_STAKE);
    env.unstake(&asset).unwrap();

    env.stake(&asset).expect("re-stake failed");
    let key = asset.pubkey();
    assert_eq!(env.frozen(&key), Some(true));
    assert_eq!(env.asset_attr(&key, STAKED).as_deref(), Some("true"));
    assert_eq!(
        env.asset_attr(&key, STAKED_AT),
        Some((START_TIME + MIN_STAKE).to_string())
    );
    assert_eq!(env.total_staked(&env.pool), 1);
}

#[test]
fn unstake_requires_a_staked_asset() {
    let mut env = Env::new();
    let asset = env.mint();

    assert_staking_error(env.unstake(&asset), StakingError::NotStaked);
}

// ---------------------------------------------------------- burn_staked_nft

#[test]
fn burning_a_staked_nft_pays_accrued_rewards_plus_bonus() {
    let mut env = Env::new();
    let asset = env.mint();
    env.stake(&asset).unwrap();

    // Burning is allowed even inside the minimum staking period.
    env.advance(PERIOD * 2);
    env.burn(&asset).expect("burn failed");

    assert_eq!(env.rewards_balance(), 2 * REWARD + BURN_BONUS);
    assert_eq!(env.rewards_supply(), 2 * REWARD + BURN_BONUS);
    assert_eq!(env.total_staked(&env.pool), 0);

    // Core "closes" a burned asset by shrinking it to a single `Key::Uninitialized` byte.
    let burned = env.svm.get_account(&asset.pubkey()).expect("asset account");
    assert_eq!(burned.data, [mpl_core::types::Key::Uninitialized as u8]);
}

#[test]
fn burn_requires_a_staked_asset() {
    let mut env = Env::new();
    let asset = env.mint();

    assert_staking_error(env.burn(&asset), StakingError::NotStaked);
    assert_eq!(env.rewards_balance(), 0);
}

#[test]
fn total_staked_tracks_multiple_assets() {
    let mut env = Env::new();
    let (a, b) = (env.mint(), env.mint());
    env.stake(&a).unwrap();
    env.stake(&b).unwrap();
    assert_eq!(env.total_staked(&env.pool), 2);

    env.advance(MIN_STAKE);
    env.unstake(&a).unwrap();
    assert_eq!(env.total_staked(&env.pool), 1);
    env.burn(&b).unwrap();
    assert_eq!(env.total_staked(&env.pool), 0);
}
