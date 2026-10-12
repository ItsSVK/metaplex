use anchor_lang::prelude::*;

#[error_code]
pub enum StakingError {
    #[msg("The reward period and minimum stake time must be valid")]
    InvalidPeriod,
    #[msg("The reward rate must be greater than zero")]
    InvalidRate,
    #[msg("Account is not a Metaplex Core asset")]
    InvalidAsset,
    #[msg("Account is not a Metaplex Core collection")]
    InvalidCollection,
    #[msg("Signer does not own this asset")]
    NotOwner,
    #[msg("Asset does not belong to this staking collection")]
    WrongCollection,
    #[msg("Asset is already staked or carries a freeze delegate")]
    AlreadyStaked,
    #[msg("Asset is not staked")]
    NotStaked,
    #[msg("Asset is still inside its minimum staking period")]
    StakeLocked,
    #[msg("No full reward period has passed since the last claim")]
    NothingToClaim,
    #[msg("A staking attribute is missing or malformed")]
    InvalidAttribute,
    #[msg("Arithmetic overflow")]
    Overflow,
}
