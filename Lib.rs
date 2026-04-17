use anchor_lang::prelude::*;
use anchor_lang::solana_program::pubkey;
use pyth_sdk_solana::load_price_feed_from_account_info;

declare_id!("11111111111111111111111111111111"); // Replace after build

pub const MAXIMUM_AGE: i64 = 34;
pub const WITHDRAWAL_LOCK_SECONDS: i64 = 7_689_600; // 89 days
pub const MIN_SLOTS_BETWEEN_RISK_TICKS: u64 = 13;
pub const MAX_DRAWDOWN_BPS_MAX: u64 = 10_000;
pub const DEFAULT_MAX_DRAWDOWN_BPS: u64 = 1500;
pub const DEFAULT_MAX_LEVERAGE: u64 = 13;
pub const MEV_MAX_PRICE_CHANGE_BPS: u64 = 200;
pub const CROSS_CHECK_MAX_DIFF_BPS: u64 = 50;
pub const PHOENIX_COOLDOWN_SECONDS: i64 = 7200;

const EXPECTED_PYTH_FEED_PUBKEY: Pubkey = pubkey!("J83w4HKfqxwcq3BEMMkPFSppX3gqekLyLJBexebFVkix");
const EXPECTED_PYTH_FEED_PUBKEY_2: Pubkey = pubkey!("H6ARHf6YXhGYeQfUzQNGk6rDNnFQfFpD2Dd4q5nZ9QxL");

#[program]
pub mod golden_governor {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, params: InitializeParams) -> Result<()> {
        require!(params.max_drawdown_bps <= MAX_DRAWDOWN_BPS_MAX, ErrorCode::InvalidParameter);

        let effective_drawdown = if params.max_drawdown_bps > DEFAULT_MAX_DRAWDOWN_BPS {
            DEFAULT_MAX_DRAWDOWN_BPS
        } else {
            params.max_drawdown_bps
        };

        let governor = &mut ctx.accounts.governor;
        let clock = Clock::get()?;

        governor.authority = ctx.accounts.user.key();
        governor.mode = GovernorMode::Normal;
        governor.max_leverage = params.max_leverage.min(DEFAULT_MAX_LEVERAGE);
        governor.max_exposure = params.max_exposure;
        governor.max_drawdown_bps = effective_drawdown;
        governor.current_equity = params.initial_equity;
        governor.peak_equity = params.initial_equity;
        governor.current_drawdown = 0;
        governor.last_action_slot = clock.slot;
        governor.last_sense_timestamp = clock.unix_timestamp;
        governor.expected_spread = params.expected_spread;
        governor.init_timestamp = if params.init_timestamp != 0 { params.init_timestamp } else { clock.unix_timestamp };
        governor.integrity_hash = 0;
        governor.integrity_salt = clock.slot;
        governor.watchdog_nonce = 0;
        governor.last_price = 0;
        governor.last_risk_tick_slot = 0;
        governor.policy_version = 1;
        governor.lockout_timestamp = 0;
        governor.strategy_flags = 0;

        emit_state_changed(governor);
        Ok(())
    }

    pub fn execute_golden_trade(
        ctx: Context<ExecuteTrade>,
        proposed_price: u64,
        equity_delta: i64,
        leverage_used: u64,
    ) -> Result<()> {
        zt_lite_verify(&ctx.accounts.governor, ctx.accounts.authority.key(), false)?;
        let clock = Clock::get()?;

        let price1 = get_pyth_price(&ctx.accounts.price_update_pyth, clock.unix_timestamp, EXPECTED_PYTH_FEED_PUBKEY)?;
        let price2 = get_pyth_price(&ctx.accounts.price_update_switchboard, clock.unix_timestamp, EXPECTED_PYTH_FEED_PUBKEY_2)?;

        let price_diff_bps = if price1 > price2 {
            ((price1 - price2) * 10_000) / price1
        } else {
            ((price2 - price1) * 10_000) / price2
        };

        if price_diff_bps > CROSS_CHECK_MAX_DIFF_BPS {
            emit_envelope_violation(8, &ctx.accounts.governor, clock.unix_timestamp);
            return err!(ErrorCode::OracleCrossCheckFailed);
        }

        let current_market_price = (price1 + price2) / 2;
        let governor = &mut ctx.accounts.governor;

        let elapsed = clock.unix_timestamp.saturating_sub(governor.last_sense_timestamp);
        require!(elapsed <= 3, ErrorCode::SafetyKernelTimeout);

        if governor.last_price > 0 {
            let price_change_bps = if current_market_price > governor.last_price {
                ((current_market_price - governor.last_price) * 10_000) / governor.last_price
            } else {
                ((governor.last_price - current_market_price) * 10_000) / governor.last_price
            };
            if price_change_bps > MEV_MAX_PRICE_CHANGE_BPS {
                emit_envelope_violation(9, governor, clock.unix_timestamp);
                return err!(ErrorCode::MEVPriceSpike);
            }
        }

        let price_diff = if current_market_price > proposed_price {
            current_market_price - proposed_price
        } else {
            proposed_price - current_market_price
        };
        let max_friction = governor.expected_spread
            .checked_mul(1618).ok_or(ErrorCode::ArithmeticOverflow)?
            .checked_div(1000).ok_or(ErrorCode::ArithmeticOverflow)?;
        require!(price_diff <= max_friction, ErrorCode::MechanicalTorqueSlip);

        match governor.mode {
            GovernorMode::Normal => {},
            GovernorMode::Degraded => {
                if equity_delta > 0 && governor.current_equity > governor.max_exposure / 2 {
                    return err!(ErrorCode::DegradedModeViolation);
                }
            },
            GovernorMode::Recovering => {
                if equity_delta > 0 { return err!(ErrorCode::RecoveringModeOnlyReductions); }
            },
            GovernorMode::Lockout => return err!(ErrorCode::LockoutMode),
        }
        require!(governor.current_equity > 0, ErrorCode::ZeroEquityViolation);
        require!(leverage_used <= governor.max_leverage, ErrorCode::LeverageLimitExceeded);

        let new_equity = (governor.current_equity as i128)
            .checked_add(equity_delta as i128)
            .ok_or(ErrorCode::ArithmeticOverflow)?;
        let new_equity_u64 = u64::try_from(new_equity).map_err(|_| error!(ErrorCode::ArithmeticOverflow))?;

        require!(new_equity_u64 <= governor.max_exposure, ErrorCode::ExposureLimitExceeded);

        if new_equity_u64 > governor.peak_equity {
            governor.peak_equity = new_equity_u64;
        }

        let drawdown_bps = if governor.peak_equity > 0 {
            let numerator = (governor.peak_equity - new_equity_u64) as u128;
            let denominator = governor.peak_equity as u128;
            (numerator.checked_mul(10_000).ok_or(ErrorCode::ArithmeticOverflow)?
                .checked_div(denominator).ok_or(ErrorCode::ArithmeticOverflow)?) as u64
        } else { 0 };

        if drawdown_bps >= governor.max_drawdown_bps {
            governor.current_drawdown = drawdown_bps;
            governor.mode = GovernorMode::Lockout;
            governor.lockout_timestamp = clock.unix_timestamp;
            governor.integrity_hash = governor.compute_integrity_hash();
            emit_state_changed(governor);
            emit_envelope_violation(7, governor, clock.unix_timestamp);
            return err!(ErrorCode::LockoutMode);
        }

        let seventy_five_pct = governor.max_drawdown_bps
            .checked_mul(75).ok_or(ErrorCode::ArithmeticOverflow)?
            .checked_div(100).ok_or(ErrorCode::ArithmeticOverflow)?;
        let fifty_pct = governor.max_drawdown_bps
            .checked_mul(50).ok_or(ErrorCode::ArithmeticOverflow)?
            .checked_div(100).ok_or(ErrorCode::ArithmeticOverflow)?;

        governor.current_drawdown = drawdown_bps;
        if drawdown_bps >= seventy_five_pct {
            governor.mode = GovernorMode::Recovering;
        } else if drawdown_bps >= fifty_pct {
            governor.mode = GovernorMode::Degraded;
        } else {
            governor.mode = GovernorMode::Normal;
        }

        governor.current_equity = new_equity_u64;
        governor.last_action_slot = clock.slot;
        governor.last_sense_timestamp = clock.unix_timestamp;
        governor.last_price = current_market_price;
        governor.integrity_hash = governor.compute_integrity_hash();

        emit_state_changed(governor);
        Ok(())
    }

    pub fn risk_tick(ctx: Context<RiskTick>) -> Result<()> {
        let governor = &mut ctx.accounts.governor;
        let clock = Clock::get()?;

        if clock.slot < governor.last_risk_tick_slot.saturating_add(MIN_SLOTS_BETWEEN_RISK_TICKS) {
            return err!(ErrorCode::RateLimitExceeded);
        }
        governor.last_risk_tick_slot = clock.slot;

        zt_lite_verify(governor, ctx.accounts.authority.key(), false)?;
        require!(governor.mode != GovernorMode::Lockout, ErrorCode::LockoutMode);

        if governor.current_equity > governor.peak_equity {
            governor.peak_equity = governor.current_equity;
        }

        let drawdown_bps = if governor.peak_equity > 0 {
            let numerator = (governor.peak_equity - governor.current_equity) as u128;
            let denominator = governor.peak_equity as u128;
            (numerator.checked_mul(10_000).ok_or(ErrorCode::ArithmeticOverflow)?
                .checked_div(denominator).ok_or(ErrorCode::ArithmeticOverflow)?) as u64
        } else { 0 };

        let seventy_five_pct = governor.max_drawdown_bps
            .checked_mul(75).ok_or(ErrorCode::ArithmeticOverflow)?
            .checked_div(100).ok_or(ErrorCode::ArithmeticOverflow)?;
        let fifty_pct = governor.max_drawdown_bps
            .checked_mul(50).ok_or(ErrorCode::ArithmeticOverflow)?
            .checked_div(100).ok_or(ErrorCode::ArithmeticOverflow)?;

        governor.current_drawdown = drawdown_bps;
        if drawdown_bps >= governor.max_drawdown_bps {
            governor.mode = GovernorMode::Lockout;
            governor.lockout_timestamp = clock.unix_timestamp;
            emit_envelope_violation(7, governor, clock.unix_timestamp);
        } else if drawdown_bps >= seventy_five_pct {
            governor.mode = GovernorMode::Recovering;
        } else if drawdown_bps >= fifty_pct {
            governor.mode = GovernorMode::Degraded;
        } else {
            governor.mode = GovernorMode::Normal;
        }

        governor.last_sense_timestamp = clock.unix_timestamp;
        governor.last_action_slot = clock.slot;
        governor.integrity_hash = governor.compute_integrity_hash();

        emit_state_changed(governor);
        Ok(())
    }

    pub fn watchdog_tick(ctx: Context<WatchdogTick>) -> Result<()> {
        let governor = &mut ctx.accounts.governor;
        let clock = Clock::get()?;

        if clock.slot < governor.last_risk_tick_slot.saturating_add(MIN_SLOTS_BETWEEN_RISK_TICKS) {
            return err!(ErrorCode::RateLimitExceeded);
        }
        governor.last_risk_tick_slot = clock.slot;

        let new_nonce = governor.watchdog_nonce.wrapping_add(1);
        require!(new_nonce > governor.watchdog_nonce, ErrorCode::WatchdogNonceReplay);
        governor.watchdog_nonce = new_nonce;

        let price1 = get_pyth_price(&ctx.accounts.price_update_pyth, clock.unix_timestamp, EXPECTED_PYTH_FEED_PUBKEY)?;
        let price2 = get_pyth_price(&ctx.accounts.price_update_switchboard, clock.unix_timestamp, EXPECTED_PYTH_FEED_PUBKEY_2)?;
        let oracle_price = (price1 + price2) / 2;

        let last = governor.last_price;
        if last > 0 {
            let percent_move = if oracle_price > last {
                ((oracle_price - last) * 10_000) / last
            } else {
                ((last - oracle_price) * 10_000) / last
            };
            if percent_move > 2_000 {
                governor.mode = GovernorMode::Lockout;
                governor.lockout_timestamp = clock.unix_timestamp;
                emit_envelope_violation(9, governor, clock.unix_timestamp);
            }
        }

        governor.last_price = oracle_price;

        let computed = governor.compute_integrity_hash();
        if governor.integrity_hash != 0 && computed != governor.integrity_hash {
            governor.mode = GovernorMode::Lockout;
            governor.lockout_timestamp = clock.unix_timestamp;
            emit_envelope_violation(0, governor, clock.unix_timestamp);
        }
        governor.integrity_hash = computed;

        emit_state_changed(governor);
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        let governor = &mut ctx.accounts.governor;
        zt_lite_verify(governor, ctx.accounts.authority.key(), false)?;

        let clock = Clock::get()?;
        let time_since_init = clock.unix_timestamp.saturating_sub(governor.init_timestamp);
        require!(time_since_init >= WITHDRAWAL_LOCK_SECONDS, ErrorCode::WithdrawalLocked);
        require!(governor.mode == GovernorMode::Normal, ErrorCode::WithdrawalModeViolation);
        require!(amount <= governor.current_equity, ErrorCode::InsufficientEquity);

        governor.current_equity = governor.current_equity.saturating_sub(amount);
        governor.last_action_slot = clock.slot;
        governor.integrity_hash = governor.compute_integrity_hash();

        emit_state_changed(governor);
        Ok(())
    }

    pub fn phoenix_restart(ctx: Context<PhoenixRestart>) -> Result<()> {
        let governor = &mut ctx.accounts.governor;
        zt_lite_verify(governor, ctx.accounts.authority.key(), true)?;

        let clock = Clock::get()?;
        require!(governor.mode == GovernorMode::Lockout, ErrorCode::PhoenixNotInLockout);
        require!(governor.lockout_timestamp != 0, ErrorCode::PhoenixCooldownNotElapsed);
        let elapsed = clock.unix_timestamp.saturating_sub(governor.lockout_timestamp);
        require!(elapsed >= PHOENIX_COOLDOWN_SECONDS, ErrorCode::PhoenixCooldownNotElapsed);

        governor.mode = GovernorMode::Recovering;
        governor.peak_equity = governor.current_equity;
        governor.current_drawdown = 0;
        governor.last_action_slot = clock.slot;
        governor.last_sense_timestamp = clock.unix_timestamp;
        governor.integrity_hash = governor.compute_integrity_hash();

        emit_state_changed(governor);
        Ok(())
    }

    pub fn upgrade_policy(ctx: Context<UpgradePolicy>, new_version: u8) -> Result<()> {
        let governor = &mut ctx.accounts.governor;
        zt_lite_verify(governor, ctx.accounts.authority.key(), true)?;
        require!(new_version > governor.policy_version, ErrorCode::InvalidParameter);
        governor.policy_version = new_version;
        governor.integrity_hash = governor.compute_integrity_hash();
        emit_state_changed(governor);
        Ok(())
    }
}

// ============================================================================
// Helper functions (outside the #[program] module)
// ============================================================================

fn get_pyth_price(
    price_update: &AccountInfo<'_>,
    current_timestamp: i64,
    expected_feed: Pubkey,
) -> Result<u64> {
    require!(price_update.key() == expected_feed, ErrorCode::OracleFeedIdInvalid);

    let price_feed = load_price_feed_from_account_info(price_update)
        .map_err(|_| error!(ErrorCode::OracleFeedIdInvalid))?;

    let pyth_price = price_feed
        .get_price_no_older_than(
            current_timestamp.try_into().unwrap(),
            MAXIMUM_AGE.try_into().unwrap(),
        )
        .ok_or(error!(ErrorCode::SafetyKernelTimeout))?;

    require!(pyth_price.price >= 0, ErrorCode::OracleNegativePrice);
    Ok(pyth_price.price as u64)
}

fn zt_lite_verify(governor: &GoldenGovernor, authority: Pubkey, allow_lockout: bool) -> Result<()> {
    if !allow_lockout && governor.mode == GovernorMode::Lockout {
        return err!(ErrorCode::LockoutMode);
    }
    require!(governor.policy_version == 1, ErrorCode::PolicyVersionMismatch);
    let computed = governor.compute_integrity_hash();
    if governor.integrity_hash != 0 && computed != governor.integrity_hash {
        return err!(ErrorCode::ZTVerificationFailed);
    }
    Ok(())
}

fn emit_state_changed(governor: &GoldenGovernor) {
    emit!(GovernorStateChanged {
        policy_version: governor.policy_version,
        mode: governor.mode as u8,
        current_equity: governor.current_equity,
        peak_equity: governor.peak_equity,
        drawdown_bps: governor.current_drawdown,
        watchdog_nonce: governor.watchdog_nonce,
        init_timestamp: governor.init_timestamp,
        last_risk_tick_slot: governor.last_risk_tick_slot,
        strategy_flags: governor.strategy_flags,
    });
}

fn emit_envelope_violation(envelope_id: u8, governor: &GoldenGovernor, timestamp: i64) {
    emit!(EnvelopeViolation {
        envelope_id,
        mode: governor.mode as u8,
        timestamp,
    });
}

// ============================================================================
// Account structs, state, events, error codes (unchanged, all correct)
// ============================================================================

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = user, space = 8 + GoldenGovernor::LEN, seeds = [b"governor", user.key().as_ref()], bump)]
    pub governor: Account<'info, GoldenGovernor>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ExecuteTrade<'info> {
    #[account(mut, seeds = [b"governor", authority.key().as_ref()], bump, has_one = authority)]
    pub governor: Account<'info, GoldenGovernor>,
    pub authority: Signer<'info>,
    /// CHECK: Pyth oracle account, verified by load_price_feed_from_account_info
    pub price_update_pyth: AccountInfo<'info>,
    /// CHECK: Pyth oracle account, verified by load_price_feed_from_account_info
    pub price_update_switchboard: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct WatchdogTick<'info> {
    #[account(mut, seeds = [b"governor", authority.key().as_ref()], bump, has_one = authority)]
    pub governor: Account<'info, GoldenGovernor>,
    pub authority: Signer<'info>,
    /// CHECK: Pyth oracle account, verified by load_price_feed_from_account_info
    pub price_update_pyth: AccountInfo<'info>,
    /// CHECK: Pyth oracle account, verified by load_price_feed_from_account_info
    pub price_update_switchboard: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct RiskTick<'info> {
    #[account(mut, seeds = [b"governor", authority.key().as_ref()], bump, has_one = authority)]
    pub governor: Account<'info, GoldenGovernor>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut, seeds = [b"governor", authority.key().as_ref()], bump, has_one = authority)]
    pub governor: Account<'info, GoldenGovernor>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct PhoenixRestart<'info> {
    #[account(mut, seeds = [b"governor", authority.key().as_ref()], bump, has_one = authority)]
    pub governor: Account<'info, GoldenGovernor>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct UpgradePolicy<'info> {
    #[account(mut, seeds = [b"governor", authority.key().as_ref()], bump, has_one = authority)]
    pub governor: Account<'info, GoldenGovernor>,
    pub authority: Signer<'info>,
}

#[account]
pub struct GoldenGovernor {
    pub authority: Pubkey,
    pub mode: GovernorMode,
    pub max_leverage: u64,
    pub max_exposure: u64,
    pub max_drawdown_bps: u64,
    pub current_equity: u64,
    pub peak_equity: u64,
    pub current_drawdown: u64,
    pub last_action_slot: u64,
    pub last_sense_timestamp: i64,
    pub expected_spread: u64,
    pub integrity_hash: u64,
    pub integrity_salt: u64,
    pub watchdog_nonce: u64,
    pub last_price: u64,
    pub init_timestamp: i64,
    pub last_risk_tick_slot: u64,
    pub policy_version: u8,
    pub lockout_timestamp: i64,
    pub strategy_flags: u8,
}

impl GoldenGovernor {
    pub const LEN: usize = 163;

    pub fn compute_integrity_hash(&s
