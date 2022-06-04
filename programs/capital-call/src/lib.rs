use anchor_lang::prelude::*;
use anchor_lang::solana_program::clock::Clock;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::token::{self, Burn, CloseAccount, Mint, MintTo, Token, TokenAccount, Transfer};

declare_id!("HRsNi3EmPjTLwEfekPYzBQmdy5UqZ7MKmcvi5rjuHder");

pub const SEED_CAPITAL_CALL: [u8; 12] = *b"capital_call";
pub const SEED_VAULT: [u8; 5] = *b"vault";
pub const SEED_LP_TOKEN_POOL: [u8; 13] = *b"lp_token_pool";
pub const SEED_VOUCHER: [u8; 7] = *b"voucher";
pub const SEED_LP_MINT_AUTHORITY: [u8; 17] = *b"lp_mint_authority";

pub const MINT_PUBKEY: &str = "ETE5KJSyx1XitibZc9hb35AneRmCH8riJzyxr9beKtZ6";

#[program]
pub mod capital_call {
    use super::*;

    /// Initialize Config
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        ctx.accounts.config.authority = ctx.accounts.authority.key();
        ctx.accounts.config.liquidity_pool = ctx.accounts.liquidity_pool.key();
        ctx.accounts.config.lp_mint = ctx.accounts.lp_mint.key();
        ctx.accounts.config.lp_mint_authority = ctx.accounts.lp_mint_authority.key();
        ctx.accounts.config.bump = *ctx
            .bumps
            .get("lp_mint_authority")
            .ok_or_else(|| error!(CapitalCallError::BumpSeedNotInHashMap))?;
        Ok(())
    }

    /// Create new capital call
    pub fn create_capital_call(
        ctx: Context<CreateCapitalCall>,
        start_time: u64,
        duration: u64,
        capacity: u64,
        credit_outstanding: u64,
    ) -> Result<()> {
        let clock = Clock::get().map_err::<error::Error, _>(Into::into)?;
        let now = clock.unix_timestamp as u64;

        require!(start_time >= now, CapitalCallError::StartTimeMustBeInFuture);
        require!(duration > 0, CapitalCallError::DurationNonZero);
        require!(capacity > 0, CapitalCallError::CapacityNonZero);

        let capital_call = &mut ctx.accounts.capital_call;
        capital_call.config = ctx.accounts.config.key();
        capital_call.vault = ctx.accounts.vault.key();
        capital_call.lp_token_pool = ctx.accounts.lp_token_pool.key();

        capital_call.start_time = start_time;
        capital_call.end_time = start_time + duration;
        capital_call.capacity = capacity;
        capital_call.redeemed = 0;
        capital_call.allocated = 0;
        capital_call.is_lp_minted = false;

        capital_call.token_liquidity = 0;
        capital_call.lp_supply = 0;
        capital_call.credit_outstanding = credit_outstanding;

        capital_call.bump = *ctx
            .bumps
            .get("capital_call")
            .ok_or_else(|| error!(CapitalCallError::BumpSeedNotInHashMap))?;
        capital_call.vault_bump = *ctx
            .bumps
            .get("vault")
            .ok_or_else(|| error!(CapitalCallError::BumpSeedNotInHashMap))?;
        capital_call.lp_token_pool_bump = *ctx
            .bumps
            .get("lp_token_pool")
            .ok_or_else(|| error!(CapitalCallError::BumpSeedNotInHashMap))?;
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        let clock = Clock::get().map_err::<error::Error, _>(Into::into)?;
        let now = clock.unix_timestamp as u64;
        let capital_call = &ctx.accounts.capital_call;

        require!(
            now >= capital_call.start_time,
            CapitalCallError::CapitalCallNotStarted
        );
        require!(
            now < capital_call.end_time,
            CapitalCallError::CapitalCallEnded
        );
        require!(
            capital_call.capacity > capital_call.allocated,
            CapitalCallError::CapitalCallAlreadyFullyFunded
        );
        require!(amount > 0, CapitalCallError::AmountNonZero);

        // Reduce amount if this tx fills vault
        let amount = amount.min(capital_call.capacity - capital_call.allocated);

        let config = capital_call.config.key();
        let start_time = capital_call.start_time.to_le_bytes();
        let capacity = capital_call.capacity.to_le_bytes();

        let seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            config.as_ref(),
            start_time.as_ref(),
            capacity.as_ref(),
            &[ctx.accounts.capital_call.bump],
        ];

        let cpi_ctx: CpiContext<_> = ctx.accounts.into();
        token::transfer(cpi_ctx.with_signer(&[&seeds]), amount)?;

        ctx.accounts.capital_call.allocated += amount;

        let voucher = &mut ctx.accounts.voucher;
        voucher.capital_call = ctx.accounts.capital_call.key();
        voucher.authority = ctx.accounts.authority.key();
        voucher.amount = amount;
        voucher.bump = *ctx
            .bumps
            .get("voucher")
            .ok_or_else(|| error!(CapitalCallError::BumpSeedNotInHashMap))?;

        emit!(DepositEvent {
            config: ctx.accounts.capital_call.config,
            capital_call: ctx.accounts.capital_call.key(),
            authority: ctx.accounts.authority.key(),
            amount,
        });

        if ctx.accounts.capital_call.capacity == ctx.accounts.capital_call.allocated {
            emit!(CapitalFullyRaisedEvent {
                config: ctx.accounts.capital_call.config,
                capital_call: ctx.accounts.capital_call.key(),
            });
        }

        Ok(())
    }

    /// Refund tokens if capital is not raised
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let clock = Clock::get().map_err::<error::Error, _>(Into::into)?;
        let now = clock.unix_timestamp as u64;
        let capital_call = &ctx.accounts.capital_call;

        require!(
            capital_call.capacity > capital_call.allocated,
            CapitalCallError::CapitalCallIsFullyFunded
        );
        require!(
            now >= capital_call.end_time,
            CapitalCallError::CapitalCallNotEnded
        );

        let config = capital_call.config;
        let start_time = capital_call.start_time.to_le_bytes();
        let capacity = capital_call.capacity.to_le_bytes();

        let seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            config.as_ref(),
            start_time.as_ref(),
            capacity.as_ref(),
            &[ctx.accounts.capital_call.bump],
        ];

        let amount = ctx.accounts.voucher.amount;
        let cpi_ctx: CpiContext<_> = ctx.accounts.into();
        token::transfer(cpi_ctx.with_signer(&[&seeds]), amount)?;

        ctx.accounts.capital_call.redeemed += amount;

        emit!(RefundEvent {
            config: ctx.accounts.capital_call.config,
            capital_call: ctx.accounts.capital_call.key(),
            authority: ctx.accounts.authority.key(),
            amount,
        });

        Ok(())
    }

    /// Mint LP tokens if capital call raised
    /// This instruction is permissionless and doesn't fail if capital call isn't fully raised or
    /// still active.
    pub fn mint_lp_tokens(ctx: Context<MintLpTokens>) -> Result<()> {
        require!(
            ctx.accounts.lp_mint.mint_authority
                == COption::Some(ctx.accounts.lp_mint_authority.key()),
            CapitalCallError::InvalidLpMintAuthority
        );
        require!(
            ctx.accounts.lp_mint.supply > 0,
            CapitalCallError::LpTokenSupplyNonZero
        );

        // exit from instruction early if capital isn't raised or lp tokens already minted
        if ctx.accounts.capital_call.capacity != ctx.accounts.capital_call.allocated
            || ctx.accounts.capital_call.is_lp_minted
        {
            return Ok(());
        }

        ctx.accounts.capital_call.lp_supply = ctx.accounts.lp_mint.supply;
        ctx.accounts.capital_call.token_liquidity = ctx.accounts.liquidity_pool.amount;

        let minted = ctx
            .accounts
            .capital_call
            .to_lp_token(ctx.accounts.capital_call.capacity)?;

        let config_key = ctx.accounts.config.key();
        let seeds = [
            SEED_LP_MINT_AUTHORITY.as_ref(),
            config_key.as_ref(),
            &[ctx.accounts.config.bump],
        ];

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.lp_mint.to_account_info(),
                    to: ctx.accounts.lp_token_pool.to_account_info(),
                    authority: ctx.accounts.lp_mint_authority.to_account_info(),
                },
                &[&seeds],
            ),
            minted,
        )?;

        let start_time = ctx.accounts.capital_call.start_time.to_le_bytes();
        let capacity_bytes = ctx.accounts.capital_call.capacity.to_le_bytes();

        let seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            config_key.as_ref(),
            start_time.as_ref(),
            capacity_bytes.as_ref(),
            &[ctx.accounts.capital_call.bump],
        ];

        let capital = ctx.accounts.capital_call.capacity;
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault.to_account_info(),
                    to: ctx.accounts.liquidity_pool.to_account_info(),
                    authority: ctx.accounts.capital_call.to_account_info(),
                },
                &[&seeds],
            ),
            capital,
        )?;

        ctx.accounts.capital_call.is_lp_minted = true;

        emit!(LpTokensMintedEvent {
            config: ctx.accounts.config.key(),
            capital_call: ctx.accounts.capital_call.key(),
            token_liquidity: ctx.accounts.capital_call.token_liquidity,
            lp_supply: ctx.accounts.capital_call.lp_supply,
            credit_outstanding: ctx.accounts.capital_call.credit_outstanding,
            capital: ctx.accounts.capital_call.capacity,
            minted
        });

        Ok(())
    }

    pub fn claim(ctx: Context<Claim>) -> Result<()> {
        let capital_call = &ctx.accounts.capital_call;
        require!(
            capital_call.is_lp_minted,
            CapitalCallError::LpTokenNotMinted
        );

        let config_key = capital_call.config;
        let start_time = capital_call.start_time.to_le_bytes();
        let capacity = capital_call.capacity.to_le_bytes();

        let seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            config_key.as_ref(),
            start_time.as_ref(),
            capacity.as_ref(),
            &[capital_call.bump],
        ];

        let amount = ctx.accounts.voucher.amount;
        let lp_amount = capital_call.to_lp_token(amount)?;

        let cpi_ctx: CpiContext<_> = ctx.accounts.into();
        token::transfer(cpi_ctx.with_signer(&[&seeds]), lp_amount)?;

        ctx.accounts.capital_call.redeemed += amount;

        emit!(ClaimEvent {
            config: ctx.accounts.capital_call.config,
            capital_call: ctx.accounts.capital_call.key(),
            authority: ctx.accounts.authority.key(),
            amount,
            lp_amount
        });

        Ok(())
    }

    /// Close capital call and related accounts
    pub fn close(ctx: Context<CloseCapitalCall>) -> Result<()> {
        let clock = Clock::get().map_err::<error::Error, _>(Into::into)?;
        let now = clock.unix_timestamp as u64;
        let capital_call = &ctx.accounts.capital_call;

        if !capital_call.is_lp_minted {
            if now > capital_call.end_time {
                require!(
                    capital_call.allocated == capital_call.redeemed,
                    CapitalCallError::CapitalCallHasToBeFullyRefunded
                );
            }
        } else {
            require!(
                capital_call.allocated == capital_call.redeemed,
                CapitalCallError::LpTokensHasToBeFullyDistributed
            );
        }

        // Someone can transfer tokens directly to vault
        let config_key = capital_call.config;
        let start_time = capital_call.start_time.to_le_bytes();
        let capacity = capital_call.capacity.to_le_bytes();

        let seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            config_key.as_ref(),
            start_time.as_ref(),
            capacity.as_ref(),
            &[capital_call.bump],
        ];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault.to_account_info(),
                    to: ctx.accounts.destination.to_account_info(),
                    authority: ctx.accounts.capital_call.to_account_info(),
                },
                &[&seeds],
            ),
            ctx.accounts.vault.amount,
        )?;

        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.vault.to_account_info(),
                destination: ctx.accounts.receiver.to_account_info(),
                authority: ctx.accounts.capital_call.to_account_info(),
            },
            &[&seeds],
        ))?;

        // burn leftover LP tokens
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.lp_mint.to_account_info(),
                    from: ctx.accounts.lp_token_pool.to_account_info(),
                    authority: ctx.accounts.capital_call.to_account_info(),
                },
                &[&seeds],
            ),
            ctx.accounts.lp_token_pool.amount,
        )?;

        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.lp_token_pool.to_account_info(),
                destination: ctx.accounts.receiver.to_account_info(),
                authority: ctx.accounts.capital_call.to_account_info(),
            },
            &[&seeds],
        ))?;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = payer,
        space = Config::SPACE,
    )]
    pub config: Account<'info, Config>,

    /// CHECK: Only for key
    pub authority: UncheckedAccount<'info>,

    /// CHECK: Only for bump calculation
    #[account(
        seeds = [
            SEED_LP_MINT_AUTHORITY.as_ref(),
            config.key().as_ref(),
        ], bump
    )]
    pub lp_mint_authority: UncheckedAccount<'info>,

    pub lp_mint: Account<'info, Mint>,

    #[account(
        constraint = liquidity_pool.mint == MINT_PUBKEY.parse::<Pubkey>().unwrap()
    )]
    pub liquidity_pool: Account<'info, TokenAccount>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(start_time: u64, duration: u64, capacity: u64)]
pub struct CreateCapitalCall<'info> {
    #[account(
        has_one = authority,
        has_one = lp_mint,
    )]
    pub config: Box<Account<'info, Config>>,

    #[account(
        init,
        payer = payer,
        space = CapitalCall::SPACE,
        seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            config.key().as_ref(),
            start_time.to_le_bytes().as_ref(),
            capacity.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub capital_call: Box<Account<'info, CapitalCall>>,

    #[account(
        init,
        payer = payer,
        token::mint = mint,
        token::authority = capital_call,
        seeds = [
            SEED_VAULT.as_ref(),
            capital_call.key().as_ref(),
        ],
        bump
    )]
    pub vault: Box<Account<'info, TokenAccount>>,

    #[account(address = MINT_PUBKEY.parse::<Pubkey>().unwrap())]
    pub mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = payer,
        token::mint = lp_mint,
        token::authority = capital_call,
        seeds = [
            SEED_LP_TOKEN_POOL.as_ref(),
            capital_call.key().as_ref(),
        ], bump
    )]
    pub lp_token_pool: Box<Account<'info, TokenAccount>>,
    pub lp_mint: Box<Account<'info, Mint>>,

    pub authority: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(
        mut,
        seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            capital_call.config.as_ref(),
            capital_call.start_time.to_le_bytes().as_ref(),
            capital_call.capacity.to_le_bytes().as_ref(),
        ],
        bump = capital_call.bump
    )]
    pub capital_call: Account<'info, CapitalCall>,

    #[account(
        init,
        payer = authority,
        space = Voucher::SPACE,
        seeds = [
            SEED_VOUCHER.as_ref(),
            capital_call.key().as_ref(),
            authority.key().as_ref(),
        ],
        bump
    )]
    pub voucher: Account<'info, Voucher>,

    #[account(
        mut,
        seeds = [
            SEED_VAULT.as_ref(),
            capital_call.key().as_ref(),
        ],
        bump = capital_call.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut)]
    pub source: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'a, 'b, 'c, 'info> From<&mut Deposit<'info>>
    for CpiContext<'a, 'b, 'c, 'info, Transfer<'info>>
{
    fn from(accounts: &mut Deposit<'info>) -> CpiContext<'a, 'b, 'c, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: accounts.source.to_account_info(),
            to: accounts.vault.to_account_info(),
            authority: accounts.authority.to_account_info(),
        };
        let cpi_program = accounts.token_program.to_account_info();
        CpiContext::new(cpi_program, cpi_accounts)
    }
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(
        mut,
        seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            capital_call.config.as_ref(),
            capital_call.start_time.to_le_bytes().as_ref(),
            capital_call.capacity.to_le_bytes().as_ref(),
        ],
        bump = capital_call.bump
    )]
    pub capital_call: Account<'info, CapitalCall>,

    #[account(
        mut,
        close = authority,
        seeds = [
            SEED_VOUCHER.as_ref(),
            capital_call.key().as_ref(),
            authority.key().as_ref(),
        ],
        bump = voucher.bump,
        has_one = authority,
        has_one = capital_call,
    )]
    pub voucher: Account<'info, Voucher>,

    #[account(
        mut,
        seeds = [
            SEED_VAULT.as_ref(),
            capital_call.key().as_ref(),
        ],
        bump = capital_call.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut)]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

impl<'a, 'b, 'c, 'info> From<&mut Refund<'info>>
    for CpiContext<'a, 'b, 'c, 'info, Transfer<'info>>
{
    fn from(accounts: &mut Refund<'info>) -> CpiContext<'a, 'b, 'c, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: accounts.vault.to_account_info(),
            to: accounts.destination.to_account_info(),
            authority: accounts.capital_call.to_account_info(),
        };
        let cpi_program = accounts.token_program.to_account_info();
        CpiContext::new(cpi_program, cpi_accounts)
    }
}

#[derive(Accounts)]
pub struct MintLpTokens<'info> {
    #[account(
        has_one = lp_mint,
        has_one = lp_mint_authority,
        has_one = liquidity_pool,
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            capital_call.config.as_ref(),
            capital_call.start_time.to_le_bytes().as_ref(),
            capital_call.capacity.to_le_bytes().as_ref(),
        ],
        bump = capital_call.bump,
        has_one = config,
        has_one = lp_token_pool,
    )]
    pub capital_call: Account<'info, CapitalCall>,

    #[account(
        mut,
        seeds = [
            SEED_VAULT.as_ref(),
            capital_call.key().as_ref(),
        ],
        bump = capital_call.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub liquidity_pool: Account<'info, TokenAccount>,

    /// CHECK: Only for bump calculation
    #[account(
        seeds = [
            SEED_LP_MINT_AUTHORITY.as_ref(),
            config.key().as_ref(),
        ], bump = config.bump
    )]
    pub lp_mint_authority: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [
            SEED_LP_TOKEN_POOL.as_ref(),
            capital_call.key().as_ref(),
        ],
        bump = capital_call.lp_token_pool_bump
    )]
    pub lp_token_pool: Account<'info, TokenAccount>,

    #[account(mut)]
    pub lp_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(
        mut,
        seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            capital_call.config.as_ref(),
            capital_call.start_time.to_le_bytes().as_ref(),
            capital_call.capacity.to_le_bytes().as_ref(),
        ],
        bump = capital_call.bump,
        has_one = lp_token_pool,
    )]
    pub capital_call: Account<'info, CapitalCall>,

    #[account(
        mut,
        seeds = [
            SEED_LP_TOKEN_POOL.as_ref(),
            capital_call.key().as_ref()],
        bump = capital_call.lp_token_pool_bump
    )]
    pub lp_token_pool: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        close = authority,
        seeds = [
            SEED_VOUCHER.as_ref(),
            capital_call.key().as_ref(),
            authority.key().as_ref()],
        bump = voucher.bump,
        has_one = authority,
        has_one = capital_call,
    )]
    pub voucher: Account<'info, Voucher>,

    #[account(mut)]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

impl<'a, 'b, 'c, 'info> From<&mut Claim<'info>> for CpiContext<'a, 'b, 'c, 'info, Transfer<'info>> {
    fn from(accounts: &mut Claim<'info>) -> CpiContext<'a, 'b, 'c, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: accounts.lp_token_pool.to_account_info(),
            to: accounts.destination.to_account_info(),
            authority: accounts.capital_call.to_account_info(),
        };
        let cpi_program = accounts.token_program.to_account_info();
        CpiContext::new(cpi_program, cpi_accounts)
    }
}

#[derive(Accounts)]
pub struct CloseCapitalCall<'info> {
    #[account(
        has_one = authority,
        has_one = lp_mint,
    )]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        close = receiver,
        seeds = [
            SEED_CAPITAL_CALL.as_ref(),
            capital_call.config.as_ref(),
            capital_call.start_time.to_le_bytes().as_ref(),
            capital_call.capacity.to_le_bytes().as_ref(),
        ],
        bump = capital_call.bump,
        has_one = config,
        has_one = vault,
        has_one = lp_token_pool,
    )]
    pub capital_call: Box<Account<'info, CapitalCall>>,

    pub authority: Signer<'info>,

    #[account(mut)]
    pub receiver: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [
            SEED_LP_TOKEN_POOL.as_ref(),
            capital_call.key().as_ref()],
        bump = capital_call.lp_token_pool_bump
    )]
    pub lp_token_pool: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [
            SEED_VAULT.as_ref(),
            capital_call.key().as_ref()],
        bump = capital_call.vault_bump,
    )]
    pub vault: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub lp_mint: Box<Account<'info, Mint>>,

    #[account(mut)]
    pub destination: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

#[account]
pub struct Config {
    pub authority: Pubkey,
    pub liquidity_pool: Pubkey,
    pub lp_mint: Pubkey,
    pub lp_mint_authority: Pubkey,
    pub bump: u8,
}

impl Config {
    pub const SPACE: usize = 8 + std::mem::size_of::<Config>();
}

#[account]
pub struct CapitalCall {
    pub config: Pubkey,
    pub vault: Pubkey,
    pub lp_token_pool: Pubkey,

    // Start time of capital call
    pub start_time: u64,

    // End time of capital call
    pub end_time: u64,

    // Expected amount
    pub capacity: u64,

    // Allocated amount
    pub allocated: u64,

    // Redeemed or return tokens
    pub redeemed: u64,

    pub token_liquidity: u64,
    pub lp_supply: u64,
    pub credit_outstanding: u64,

    pub is_lp_minted: bool,

    pub bump: u8,
    pub vault_bump: u8,
    pub lp_token_pool_bump: u8,
}

impl CapitalCall {
    pub const SPACE: usize = 8 + std::mem::size_of::<CapitalCall>();

    pub fn to_lp_token(&self, amount: u64) -> Result<u64> {
        u64::try_from(
            amount as u128 * (self.token_liquidity as u128 + self.credit_outstanding as u128)
                / self.lp_supply as u128,
        )
        .map_err(|_| error!(CapitalCallError::CalculationError))
    }
}

#[account]
pub struct Voucher {
    pub capital_call: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
    pub bump: u8,
}

impl Voucher {
    pub const SPACE: usize = 8 + std::mem::size_of::<Voucher>();
}

#[error_code]
pub enum CapitalCallError {
    BumpSeedNotInHashMap,

    // Create Capital Call errors
    StartTimeMustBeInFuture,
    DurationNonZero,
    CapacityNonZero,

    // Deposit errors
    CapitalCallNotStarted,
    CapitalCallEnded,
    CapitalCallAlreadyFullyFunded,
    AmountNonZero,

    // Refund errors
    CapitalCallNotEnded,
    CapitalCallIsFullyFunded,

    // Mint LP Tokens
    InvalidLpMintAuthority,
    LpTokenSupplyNonZero,
    CalculationError,

    // Claim
    LpTokenNotMinted,

    // Close
    CapitalCallHasToBeFullyRefunded,
    LpTokensHasToBeFullyDistributed,
}

#[event]
pub struct DepositEvent {
    pub config: Pubkey,
    pub capital_call: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
}

#[event]
pub struct CapitalFullyRaisedEvent {
    pub config: Pubkey,
    pub capital_call: Pubkey,
}

#[event]
pub struct RefundEvent {
    pub config: Pubkey,
    pub capital_call: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
}

#[event]
pub struct LpTokensMintedEvent {
    pub config: Pubkey,
    pub capital_call: Pubkey,
    pub token_liquidity: u64,
    pub lp_supply: u64,
    pub credit_outstanding: u64,
    pub capital: u64,
    pub minted: u64,
}

#[event]
pub struct ClaimEvent {
    pub config: Pubkey,
    pub capital_call: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
    pub lp_amount: u64,
}
