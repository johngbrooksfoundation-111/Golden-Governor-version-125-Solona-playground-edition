// voltrManager.ts – High-Yield Focused Edition with Fallback Liquidation (Governor-Protected, 3s-Safe)

import * as anchor from "@coral-xyz/anchor";
import { PublicKey, Connection, Keypair } from "@solana/web3.js";

// === CONFIG ===

const RPC_URL = "https://api.devnet.solana.com";
const GOVERNOR_PROGRAM_ID = new PublicKey("11111111111111111111111111111111");
const GOVERNOR_SEED = "governor";

const authorityKeypair = Keypair.generate(); // replace with real keypair

const PYTH_PRICE_ACCOUNT = new PublicKey(
  "J83w4HKfqxwcq3BEMMkPFSppX3gqekLyLJBexebFVkix"
);

const SWITCHBOARD_PRICE_ACCOUNT = new PublicKey(
  "H6ARHf6YXhGYeQfUzQNGk6rDNnFQfFpD2Dd4q5nZ9QxL"
);

// === STRATEGIES ===

const STRATEGIES = [
  "KAMINO_MULTIPLY_SOL_JITOSOL",
  "KAMINO_RWA_MULTIPLY",
  "SOLSTICE_YIELDVAULT_LEVERAGED",
  "KAMINO_STABLE_LEVERAGE_LOOP",
  "METEORA_DYNAMIC_LEVERAGED",
  "LST_LRT_LEVERAGED_SPREAD",
  "PERP_BASIS_WITH_TILT",
  "EXPONENT_FIXED_LEVERAGED",
  "JUPITER_AUTO_COMPOUND",
  "MARINADE_JITO_LEVERAGED",
  "SOLSTICE_RWA",
  "KAMINO_VAULT",
  "STABLECOIN_BASIS",
  "MICRO_REBALANCE",
  "JITOSOL_FUNDING_HARVEST",
  "KVANTS_CARRY_MICRO",
  "AI_OPTIMIZED_LEVERAGED_REBAL",
  "PRIME_RWA_LEVERAGED_LOOP",
] as const;

type Strategy = (typeof STRATEGIES)[number];
type MarketCondition = "bull" | "bear";

// High-yield focus set
const HIGH_YIELD_STRATEGIES: Strategy[] = [
  "KAMINO_MULTIPLY_SOL_JITOSOL",
  "SOLSTICE_YIELDVAULT_LEVERAGED",
  "AI_OPTIMIZED_LEVERAGED_REBAL",
  "KAMINO_RWA_MULTIPLY",
  "PRIME_RWA_LEVERAGED_LOOP",
  "PERP_BASIS_WITH_TILT",
  "METEORA_DYNAMIC_LEVERAGED",
];

// Gross daily yields
const GROSS_YIELDS: Record<Strategy, number> = {
  KAMINO_MULTIPLY_SOL_JITOSOL: 0.0032,
  KAMINO_RWA_MULTIPLY: 0.0029,
  SOLSTICE_YIELDVAULT_LEVERAGED: 0.0025,
  KAMINO_STABLE_LEVERAGE_LOOP: 0.0022,
  METEORA_DYNAMIC_LEVERAGED: 0.0020,
  LST_LRT_LEVERAGED_SPREAD: 0.0018,
  PERP_BASIS_WITH_TILT: 0.0017,
  EXPONENT_FIXED_LEVERAGED: 0.0016,
  JUPITER_AUTO_COMPOUND: 0.0014,
  MARINADE_JITO_LEVERAGED: 0.0015,
  SOLSTICE_RWA: 0.00049,
  KAMINO_VAULT: 0.00065,
  STABLECOIN_BASIS: 0.00041,
  MICRO_REBALANCE: 0.00033,
  JITOSOL_FUNDING_HARVEST: 0.00115,
  KVANTS_CARRY_MICRO: 0.00095,
  AI_OPTIMIZED_LEVERAGED_REBAL: 0.0021,
  PRIME_RWA_LEVERAGED_LOOP: 0.0023,
};

// Fees (bps)
const GAS_FEE_BPS = 0.8;
const SLIPPAGE_BPS = 2.5;

// Rebalancing & sizing
const MAX_EQUITY_DELTA_PCT = 0.008; // 0.8% max move per tick
const BASIS_DELTA_PCT = 0.002; // 0.2% for basis overlay (unused directly)

// Leverage & risk
const TARGET_LEVERAGE_NORMAL = 13;
const TARGET_LEVERAGE_DEGRADED = 3; // aligned with SRD (3x degraded)
const TARGET_LEVERAGE_RECOVERING = 1;

// Torque envelope
const TORQUE_MULTIPLIER = 1.618;

// === GOVERNOR ACCOUNT SHAPE ===

enum GovernorMode {
  Normal = 0,
  Degraded = 1,
  Recovering = 2,
  Lockout = 3,
}

interface GoldenGovernorAccount {
  authority: PublicKey;
  mode: GovernorMode;
  max_leverage: anchor.BN;
  max_exposure: anchor.BN;
  max_drawdown_bps: anchor.BN;
  current_equity: anchor.BN;
  peak_equity: anchor.BN;
  current_drawdown: anchor.BN;
  last_action_slot: anchor.BN;
  last_sense_timestamp: anchor.BN;
  expected_spread: anchor.BN;
  integrity_hash: anchor.BN;
  integrity_salt: anchor.BN;
  watchdog_nonce: anchor.BN;
  last_price: anchor.BN;
  init_timestamp: anchor.BN;
  last_risk_tick_slot: anchor.BN;
  policy_version: number;
  lockout_timestamp: anchor.BN;
  strategy_flags: number;
}

// === MANAGER ===

export class VoltrVaultManager {
  private provider: anchor.AnchorProvider;
  private program: anchor.Program;
  private governorPda: PublicKey;

  // Fallback liquidation state (client-side only)
  private fallbackActive: boolean = false;
  private fallbackLiquidationEquity: number = 0;
  private fallbackLiquidationPrice: number = 0;
  private fallbackCooldownUntil: number = 0; // unix timestamp (seconds)

  constructor() {
    const connection = new Connection(RPC_URL, "confirmed");
    const wallet = new anchor.Wallet(authorityKeypair);

    this.provider = new anchor.AnchorProvider(connection, wallet, {
      preflightCommitment: "confirmed",
    });

    anchor.setProvider(this.provider);

    // @ts-ignore – use your real IDL here
    this.program = new anchor.Program({}, GOVERNOR_PROGRAM_ID, this.provider);

    this.governorPda = PublicKey.findProgramAddressSync(
      [Buffer.from(GOVERNOR_SEED), authorityKeypair.publicKey.toBuffer()],
      GOVERNOR_PROGRAM_ID
    )[0];
  }

  // High-yield weighting logic
  private computeStrategyWeights(
    marketCondition: MarketCondition
  ): Record<Strategy, number> {
    const weights: Record<Strategy, number> = {} as any;
    STRATEGIES.forEach((s) => (weights[s] = 0));

    const isBull = marketCondition === "bull";

    // High-yield leveraged loops (70–85%)
    const loopWeight = isBull ? 0.72 : 0.55;
    weights["KAMINO_MULTIPLY_SOL_JITOSOL"] = loopWeight * 0.65;
    weights["SOLSTICE_YIELDVAULT_LEVERAGED"] = loopWeight * 0.15;
    weights["AI_OPTIMIZED_LEVERAGED_REBAL"] = loopWeight * 0.10;
    weights["KAMINO_RWA_MULTIPLY"] = loopWeight * 0.05;
    weights["PRIME_RWA_LEVERAGED_LOOP"] = loopWeight * 0.05;

    // Dynamic / carry
    weights["METEORA_DYNAMIC_LEVERAGED"] = 0.08;
    weights["PERP_BASIS_WITH_TILT"] = 0.07;

    // Delta-neutral basis overlay
    const basisWeight = isBull ? 0.15 : 0.30;
    weights["PERP_BASIS_WITH_TILT"] += basisWeight * 0.6;

    const total = Object.values(weights).reduce((a, b) => a + b, 0) || 1;
    Object.keys(weights).forEach((k) => {
      weights[k as Strategy] = weights[k as Strategy] / total;
    });

    return weights;
  }

  private async fetchGovernor(): Promise<GoldenGovernorAccount> {
    const acct = await this.program.account.goldenGovernor.fetch(
      this.governorPda
    );
    return acct as GoldenGovernorAccount;
  }

  private modeToMaxLeverage(mode: GovernorMode, onChainMaxLev: number): number {
    let modeCap: number;
    switch (mode) {
      case GovernorMode.Normal:
        modeCap = TARGET_LEVERAGE_NORMAL;
        break;
      case GovernorMode.Degraded:
        modeCap = TARGET_LEVERAGE_DEGRADED;
        break;
      case GovernorMode.Recovering:
        modeCap = TARGET_LEVERAGE_RECOVERING;
        break;
      case GovernorMode.Lockout:
      default:
        modeCap = 0;
        break;
    }
    return Math.min(modeCap, onChainMaxLev);
  }

  // Compute drawdown in bps (mirrors on-chain logic)
  private computeDrawdownBps(governor: GoldenGovernorAccount): number {
    const peak = governor.peak_equity.toNumber();
    const current = governor.current_equity.toNumber();
    if (peak === 0) return 0;
    const num = peak - current;
    return Math.floor((num * 10_000) / peak);
  }

  // risk_tick before trade to satisfy 3s safety kernel
  private async riskTick(): Promise<void> {
    await this.program.methods
      // @ts-ignore – use your IDL method name
      .riskTick()
      .accounts({
        governor: this.governorPda,
        authority: authorityKeypair.publicKey,
      })
      .signers([authorityKeypair])
      .rpc();
  }

  public async tick(
    marketCondition: MarketCondition,
    asOf?: Date
  ): Promise<void> {
    const now = Math.floor((asOf ?? new Date()).getTime() / 1000);

    const governor = await this.fetchGovernor();
    const mode = governor.mode;

    // --- 0. If governor already in Lockout, do nothing ---
    if (mode === GovernorMode.Lockout) {
      console.log("Lockout/flat – no trades submitted.");
      return;
    }

    const currentEquity = governor.current_equity.toNumber();
    const maxDrawdownBps = governor.max_drawdown_bps.toNumber();
    const currentDrawdownBps = this.computeDrawdownBps(governor);

    const eightyFivePct = Math.floor((maxDrawdownBps * 85) / 100);

    // --- 1. Fallback trigger: forced liquidation at 85% of envelope ---
    if (
      !this.fallbackActive &&
      currentDrawdownBps >= eightyFivePct &&
      currentDrawdownBps < maxDrawdownBps
    ) {
      console.log(
        "Fallback trigger hit – forcing full liquidation to USDC at drawdown",
        currentDrawdownBps,
        "bps"
      );

      const lastPrice = governor.last_price.toNumber();
      const oraclePrice = lastPrice > 0 ? lastPrice : 100;

      // Full liquidation: move all volatile exposure to USDC
      const equityDelta = -currentEquity; // risk-reduction, green trade
      const leverageUsed = 0; // minimal leverage; governor only cares that risk is reduced

      // Record fallback state
      this.fallbackActive = true;
      this.fallbackLiquidationEquity = currentEquity;
      this.fallbackLiquidationPrice = oraclePrice;
      this.fallbackCooldownUntil = now + 7200; // 2h cooldown (PHOENIX_COOLDOWN_SECONDS)

      // Safety kernel: risk_tick then executeGoldenTrade
      await this.riskTick();

      await this.program.methods
        .executeGoldenTrade(
          new anchor.BN(Math.round(oraclePrice)),
          new anchor.BN(Math.round(equityDelta)),
          new anchor.BN(leverageUsed)
        )
        .accounts({
          governor: this.governorPda,
          authority: authorityKeypair.publicKey,
          priceUpdatePyth: PYTH_PRICE_ACCOUNT,
          priceUpdateSwitchboard: SWITCHBOARD_PRICE_ACCOUNT,
        })
        .signers([authorityKeypair])
        .rpc();

      console.log("Fallback liquidation trade submitted.");
      return;
    }

    // --- 2. Fallback cooldown: stay flat in USDC ---
    if (this.fallbackActive && now < this.fallbackCooldownUntil) {
      console.log(
        "In fallback cooldown window – staying in USDC, no trades submitted."
      );
      return;
    }

    // --- 3. Fallback re-entry after cooldown ---
    if (this.fallbackActive && now >= this.fallbackCooldownUntil) {
      const lastPrice = governor.last_price.toNumber();
      const oraclePrice = lastPrice > 0 ? lastPrice : 100;

      const liqPrice = this.fallbackLiquidationPrice;

      let equityDelta: number;
      let direction: "long" | "short";

      if (oraclePrice < liqPrice) {
        // Price below liquidation level → risk-reduction long
        direction = "long";
        equityDelta = this.fallbackLiquidationEquity * 0.3; // 30% step back in
      } else if (oraclePrice === liqPrice) {
        // At liquidation level → risk-neutral long
        direction = "long";
        equityDelta = this.fallbackLiquidationEquity * 0.3;
      } else {
        // Above liquidation level → risk-reduction short
        direction = "short";
        equityDelta = -this.fallbackLiquidationEquity * 0.3;
      }

      console.log(
        `Fallback re-entry: ${direction} at price ${oraclePrice}, delta = ${equityDelta.toFixed(
          2
        )}`
      );

      if (Math.abs(equityDelta) < 1) {
        console.log("Fallback re-entry delta too small, skipping.");
        this.fallbackActive = false;
        return;
      }

      await this.riskTick();

      const leverageUsed = this.modeToMaxLeverage(
        governor.mode,
        governor.max_leverage.toNumber()
      );

      await this.program.methods
        .executeGoldenTrade(
          new anchor.BN(Math.round(oraclePrice)),
          new anchor.BN(Math.round(equityDelta)),
          new anchor.BN(leverageUsed)
        )
        .accounts({
          governor: this.governorPda,
          authority: authorityKeypair.publicKey,
          priceUpdatePyth: PYTH_PRICE_ACCOUNT,
          priceUpdateSwitchboard: SWITCHBOARD_PRICE_ACCOUNT,
        })
        .signers([authorityKeypair])
        .rpc();

      console.log("Fallback re-entry trade submitted.");

      // End of fallback cycle
      this.fallbackActive = false;
      this.fallbackLiquidationEquity = 0;
      this.fallbackLiquidationPrice = 0;
      this.fallbackCooldownUntil = 0;

      return;
    }

    // --- 4. Normal manager behavior ---

    const onChainMaxLev = governor.max_leverage.toNumber();
    const maxLevCap = this.modeToMaxLeverage(mode, onChainMaxLev);

    console.log(
      "Governor mode:",
      GovernorMode[mode],
      "modeCap:",
      maxLevCap,
      "onChainMaxLev:",
      onChainMaxLev
    );

    if (maxLevCap === 0) {
      console.log("Leverage cap is zero – no trades submitted.");
      return;
    }

    const weights = this.computeStrategyWeights(marketCondition);

    // 1. Blended daily yield from high-yield strategies
    let dailyYield = 0;
    for (const s of HIGH_YIELD_STRATEGIES) {
      dailyYield += weights[s] * GROSS_YIELDS[s];
    }

    // 2. Delta-neutral basis boost (torque envelope)
    const lastPrice2 = governor.last_price.toNumber();
    const oraclePrice2 = lastPrice2 > 0 ? lastPrice2 : 100; // governor will re-check on-chain
    const proposedPrice = oraclePrice2; // we propose close to oracle
    const spread = Math.abs(proposedPrice - oraclePrice2);
    const expectedSpread = governor.expected_spread.toNumber();
    const torqueThreshold = expectedSpread * TORQUE_MULTIPLIER;

    if (spread > torqueThreshold) {
      const basisBoost =
        (0.12 / 365) * (weights["PERP_BASIS_WITH_TILT"] || 0); // ~12% annualized basis
      dailyYield += basisBoost;
    }

    // 3. Subtract fees
    const fees = (GAS_FEE_BPS + SLIPPAGE_BPS) / 10_000;
    dailyYield = Math.max(0, dailyYield - fees);

    // Conservative scaling
    let equityDelta = currentEquity * dailyYield * 0.7;

    // Mode-based cases
    if (mode === GovernorMode.Recovering) {
      console.log("Recovering mode – forcing small reduction.");
      equityDelta = -Math.abs(equityDelta * 0.3);
    } else if (mode === GovernorMode.Degraded) {
      const maxDegradedMove = currentEquity * 0.003;
      equityDelta = Math.min(equityDelta, maxDegradedMove);
    }

    // Cap move size
    const maxMove = currentEquity * MAX_EQUITY_DELTA_PCT;
    if (equityDelta > maxMove) equityDelta = maxMove;
    if (equityDelta < -maxMove) equityDelta = -maxMove;

    console.log("Daily yield (net):", (dailyYield * 100).toFixed(4) + "%");
    console.log("Equity delta:", equityDelta.toFixed(2));

    if (Math.abs(equityDelta) < 1) {
      console.log("Equity delta too small, skipping trade.");
      return;
    }

    await this.riskTick(); // 3s safety kernel

    const leverageUsed2 = maxLevCap;

    await this.program.methods
      .executeGoldenTrade(
        new anchor.BN(Math.round(oraclePrice2)),
        new anchor.BN(Math.round(equityDelta)),
        new anchor.BN(leverageUsed2)
      )
      .accounts({
        governor: this.governorPda,
        authority: authorityKeypair.publicKey,
        priceUpdatePyth: PYTH_PRICE_ACCOUNT,
        priceUpdateSwitchboard: SWITCHBOARD_PRICE_ACCOUNT,
      })
      .signers([authorityKeypair])
      .rpc();

    console.log("execute_golden_trade submitted.");
  }
}

// Simple runner (example)

async function main() {
  const mgr = new VoltrVaultManager();
  for (let i = 0; i < 10; i++) {
    const marketCondition: MarketCondition =
      Math.random() < 0.65 ? "bull" : "bear";
    console.log(`\n=== TICK ${i} (${marketCondition.toUpperCase()}) ===`);
    await mgr.tick(marketCondition);
  }
}

main().catch((e) => console.error(e));

