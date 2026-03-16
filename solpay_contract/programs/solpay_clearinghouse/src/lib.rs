use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("FPbAoy16bcjkAQmgrbwzjRA7GsKYEGmZyRBFChhDztXx");

/// Fee basis points per tier (100 bp = 1%)
/// $0-$10     → 1.00% = 100 bp
/// $10-$50    → 0.75% =  75 bp
/// $50-$100   → 0.50% =  50 bp
/// $100+      → 0.25% =  25 bp
fn calculate_fee_bps(usdc_amount: u64) -> u64 {
    // USDC has 6 decimals, so $10 = 10_000_000
    let ten = 10_000_000u64;
    let fifty = 50_000_000u64;
    let hundred = 100_000_000u64;

    if usdc_amount >= hundred {
        25
    } else if usdc_amount >= fifty {
        50
    } else if usdc_amount >= ten {
        75
    } else {
        100
    }
}

#[program]
pub mod solpay_clearinghouse {
    use super::*;

    /// Initialize the clearinghouse. Creates the global config PDA and the pool
    /// token account (ATA owned by the pool PDA) that acts as the privacy buffer.
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.authority = ctx.accounts.authority.key();
        config.fee_collector = ctx.accounts.fee_collector.key();
        config.usdc_mint = ctx.accounts.usdc_mint.key();
        config.pool_bump = ctx.bumps.pool_authority;
        config.config_bump = ctx.bumps.config;
        config.total_processed = 0;
        config.total_fees_collected = 0;
        config.transaction_count = 0;

        msg!("SolPay Clearinghouse initialized");
        msg!("Authority: {}", config.authority);
        msg!("Fee collector: {}", config.fee_collector);
        Ok(())
    }

    /// Process a payment from a customer to a merchant.
    /// Flow: Customer → Pool (privacy buffer) → Merchant (minus fee) + Fee Collector
    ///
    /// The customer signs this transaction via their wallet after scanning the QR / Blink.
    /// The pool PDA acts as an intermediary so neither party sees the other's full balance.
    pub fn process_payment(
        ctx: Context<ProcessPayment>,
        amount: u64,
        payment_id: [u8; 16],
    ) -> Result<()> {
        require!(amount > 0, SolPayError::ZeroAmount);

        let fee_bps = calculate_fee_bps(amount);
        let fee_amount = amount
            .checked_mul(fee_bps)
            .unwrap()
            .checked_div(10_000)
            .unwrap();
        let merchant_amount = amount.checked_sub(fee_amount).unwrap();

        // 1. Transfer full amount from customer to pool (privacy buffer)
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.customer_token_account.to_account_info(),
                    to: ctx.accounts.pool_token_account.to_account_info(),
                    authority: ctx.accounts.customer.to_account_info(),
                },
            ),
            amount,
        )?;

        // Derive pool signer seeds
        let config_key = ctx.accounts.config.key();
        let pool_seeds: &[&[u8]] = &[
            b"pool",
            config_key.as_ref(),
            &[ctx.accounts.config.pool_bump],
        ];
        let signer_seeds = &[pool_seeds];

        // 2. Transfer merchant's share from pool to merchant
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.pool_token_account.to_account_info(),
                    to: ctx.accounts.merchant_token_account.to_account_info(),
                    authority: ctx.accounts.pool_authority.to_account_info(),
                },
                signer_seeds,
            ),
            merchant_amount,
        )?;

        // 3. Transfer fee from pool to fee collector
        if fee_amount > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.pool_token_account.to_account_info(),
                        to: ctx.accounts.fee_collector_token_account.to_account_info(),
                        authority: ctx.accounts.pool_authority.to_account_info(),
                    },
                    signer_seeds,
                ),
                fee_amount,
            )?;
        }

        // Update config stats
        let config = &mut ctx.accounts.config;
        config.total_processed = config.total_processed.checked_add(amount).unwrap();
        config.total_fees_collected = config
            .total_fees_collected
            .checked_add(fee_amount)
            .unwrap();
        config.transaction_count = config.transaction_count.checked_add(1).unwrap();

        // Emit event for indexing
        emit!(PaymentProcessed {
            payment_id,
            customer: ctx.accounts.customer.key(),
            merchant: ctx.accounts.merchant_token_account.key(),
            amount,
            fee: fee_amount,
            merchant_received: merchant_amount,
            fee_bps,
            timestamp: Clock::get()?.unix_timestamp,
        });

        msg!(
            "Payment processed: {} USDC (fee: {} USDC, merchant: {} USDC)",
            amount,
            fee_amount,
            merchant_amount
        );

        Ok(())
    }
}

// ─── Accounts ───

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + ClearinghouseConfig::INIT_SPACE,
        seeds = [b"config"],
        bump
    )]
    pub config: Account<'info, ClearinghouseConfig>,

    /// The PDA that owns the pool token account
    /// CHECK: PDA derived from seeds, used as token account authority
    #[account(
        seeds = [b"pool", config.key().as_ref()],
        bump
    )]
    pub pool_authority: UncheckedAccount<'info>,

    /// Pool's USDC token account (ATA of pool_authority)
    #[account(
        init,
        payer = authority,
        associated_token::mint = usdc_mint,
        associated_token::authority = pool_authority,
    )]
    pub pool_token_account: Account<'info, TokenAccount>,

    /// The wallet that receives platform fees
    /// CHECK: Can be any valid pubkey, validated by authority
    pub fee_collector: UncheckedAccount<'info>,

    pub usdc_mint: Account<'info, Mint>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(amount: u64, payment_id: [u8; 16])]
pub struct ProcessPayment<'info> {
    /// The customer paying
    #[account(mut)]
    pub customer: Signer<'info>,

    /// Clearinghouse config
    #[account(
        mut,
        seeds = [b"config"],
        bump = config.config_bump,
    )]
    pub config: Account<'info, ClearinghouseConfig>,

    /// Pool PDA authority
    /// CHECK: Derived from seeds, validated by bump
    #[account(
        seeds = [b"pool", config.key().as_ref()],
        bump = config.pool_bump,
    )]
    pub pool_authority: UncheckedAccount<'info>,

    /// Pool's USDC token account
    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = pool_authority,
    )]
    pub pool_token_account: Account<'info, TokenAccount>,

    /// Customer's USDC token account
    #[account(
        mut,
        constraint = customer_token_account.owner == customer.key() @ SolPayError::InvalidTokenOwner,
        constraint = customer_token_account.mint == usdc_mint.key() @ SolPayError::InvalidMint,
    )]
    pub customer_token_account: Account<'info, TokenAccount>,

    /// Merchant's USDC token account (receives payment minus fee)
    #[account(
        mut,
        constraint = merchant_token_account.mint == usdc_mint.key() @ SolPayError::InvalidMint,
    )]
    pub merchant_token_account: Account<'info, TokenAccount>,

    /// Fee collector's USDC token account
    #[account(
        mut,
        constraint = fee_collector_token_account.mint == usdc_mint.key() @ SolPayError::InvalidMint,
        constraint = fee_collector_token_account.key() == config.fee_collector @ SolPayError::InvalidFeeCollector,
    )]
    pub fee_collector_token_account: Account<'info, TokenAccount>,

    pub usdc_mint: Account<'info, Mint>,
    pub token_program: Program<'info, Token>,
}

// ─── State ───

#[account]
#[derive(InitSpace)]
pub struct ClearinghouseConfig {
    pub authority: Pubkey,
    pub fee_collector: Pubkey,
    pub usdc_mint: Pubkey,
    pub pool_bump: u8,
    pub config_bump: u8,
    pub total_processed: u64,
    pub total_fees_collected: u64,
    pub transaction_count: u64,
}

// ─── Events ───

#[event]
pub struct PaymentProcessed {
    pub payment_id: [u8; 16],
    pub customer: Pubkey,
    pub merchant: Pubkey,
    pub amount: u64,
    pub fee: u64,
    pub merchant_received: u64,
    pub fee_bps: u64,
    pub timestamp: i64,
}

// ─── Errors ───

#[error_code]
pub enum SolPayError {
    #[msg("Payment amount must be greater than zero")]
    ZeroAmount,
    #[msg("Invalid token account owner")]
    InvalidTokenOwner,
    #[msg("Invalid token mint")]
    InvalidMint,
    #[msg("Invalid fee collector account")]
    InvalidFeeCollector,
}
