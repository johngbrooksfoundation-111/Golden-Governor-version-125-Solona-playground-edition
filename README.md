README
Golden Governor v1.2.5 Lite + Voltr Vault Manager
Build‑A‑Bear Hackathon 2026 Submission
Author: John G. Brooks Foundation / Ranger Finance
Date: April 16, 2026

1. OVERVIEW
This repository contains:
- Golden Governor v1.2.5 Lite (on‑chain safety kernel, Rust/Anchor)
- Voltr Vault Manager (off‑chain orchestrator, TypeScript)
- Fallback liquidation logic
- Mode‑aware leverage control
- Zero‑trust, safety‑critical architecture

The system enforces a hard 15% maximum drawdown and uses a 10‑envelope safety kernel to validate every trade.

2. FEATURES
- Hard 15% drawdown cap (on‑chain enforced)
- 10 independent safety envelopes
- Dual oracle cross‑check (Pyth + Switchboard)
- MEV spike detection (2% trade‑time, 20% watchdog)
- 3‑second safety window
- Phoenix Restart after 2‑hour lockout
- Fallback liquidation at 85% of DD envelope
- Risk‑reduction re‑entry logic (long/short)
- Deterministic yield engine

3. FILE STRUCTURE
/governor/
    lib.rs (Golden Governor v1.2.5 Lite)
/manager/
    voltrManager.ts (off‑chain orchestrator)

4. INSTALLATION
Prerequisites:
- Node.js 18+
- Anchor CLI
- Solana CLI
- Yarn or npm

Install dependencies:
    yarn install
or
    npm install

5. RUNNING THE MANAGER
To execute 10 ticks:
    ts-node voltrManager.ts

The manager:
- Fetches governor state
- Calls risk_tick()
- Computes yield
- Applies mode‑aware leverage
- Executes trades through executeGoldenTrade()
- Triggers fallback liquidation when needed

6. FALLBACK LIQUIDATION LOGIC
Trigger:
    drawdown ≥ 85% of max_drawdown_bps (12.75% for 15% cap)

Actions:
- Full liquidation to USDC
- Record liquidation price
- Enter 7200‑second cooldown

Re‑Entry:
- Long if price < liquidation price
- Long if price = liquidation price
- Short if price > liquidation price

7. GOVERNOR SAFETY ENVELOPES
E‑0 Integrity Hash
E‑1 Oracle Age ≤ 34s
E‑2 Torque/Slippage ≤ spread × 1.618
E‑3 Mode Gate
E‑4 Zero Equity
E‑5 Leverage Cap ≤ 13x
E‑6 Exposure Cap
E‑7 Drawdown Cap 15%
E‑8 Oracle Cross‑Check ≤ 50 bps
E‑9 MEV Spike ≤ 2%

8. MODE LADDER
Normal: < 50% DD
Degraded: 50–75%
Recovering: 75–100%
Lockout: ≥ 100%

9. PHOENIX RESTART
After 7200 seconds in Lockout:
- Mode → Recovering
- peak_equity = current_equity
- drawdown = 0

10. DEVELOPMENT NOTES
- All arithmetic on‑chain uses checked math.
- No floating‑point operations on‑chain.
- Manager must call risk_tick() before every trade.
- Manager must halt trading during Lockout.

11. DISCLAIMER
This is a hackathon demo. Not production code. No financial advice.

END OF README

The system is designed to demonstrate:

- Deterministic, safety‑critical financial control  
- Strict separation of authority between on‑chain and off‑chain components  
- Zero‑trust Lite integrity enforcement  
- Multi‑oracle safety envelopes  
- Mode ladder transitions (Normal → Degraded → Recovering → Lockout)  
- Phoenix lifecycle for safe recovery  
- Advisory‑only ML/DML (no autonomous authority)  
- Full offline reproducibility via deterministic test harness  

This documentation set is structured to meet the expectations of a safety‑critical engineering review.

---

## System Components

### **1. Golden Governor Lite v1.25 (On‑Chain Safety Kernel)**  
A minimal, deterministic, safety‑critical Solana program enforcing:

- Oracle age limits  
- Torque/slippage envelope  
- Leverage ceilings  
- Exposure ceilings  
- Drawdown ceilings  
- Multi‑oracle divergence checks  
- MEV rejection & lockout  
- Zero‑equity protection  
- ZT Lite integrity hashing  
- Phoenix cooldown & restart  

GG Lite v1.25 is the **final authority**.  
All off‑chain components must comply with its envelopes.

---

### **2. Voltr Vault Manager (Off‑Chain Deterministic Controller)**  
A deterministic controller that:

- Reads on‑chain state  
- Computes leverage targets based on mode ladder  
- Constructs proposed trades  
- Logs every decision deterministically  
- Delegates final authority to GOFAI  
- Submits trades only when envelopes allow  

The Manager never overrides on‑chain safety.

---

### **3. Envelope‑0 GOFAI (Deterministic Rule Engine)**  
A deterministic arbiter implementing:

- Rules A–K  
- Howey classification  
- Deterministic decision outcomes:  
  - allow  
  - deny  
  - degrade  
  - lockout  
  - human_review  

GOFAI is the **final off‑chain arbiter** before any trade is attempted.

---

### **4. ML/DML Advisory Modules**  
These modules provide **advisory‑only** signals:

- ML: deterministic scoring of volatility, liquidity, funding, correlation  
- DML: deterministic pattern classification (normal, regime shift, liquidity crunch, manipulation)

They **cannot** override GOFAI or GG Lite.

---

### **5. Deterministic Test Harness**  
A complete offline simulator that:

- Replays market sequences deterministically  
- Enforces the same envelopes as GG Lite  
- Logs every state transition  
- Validates every requirement in the SRD  
- Produces reproducible evidence for the BTM  

This ensures full transparency and auditability.

---

## Documentation Included in This Repository

### **SAD001V125LTSE**  
Unified System Architecture Document  
Describes the entire integrated system at the architectural level.

### **SRD001V125LTSE**  
Unified Software Requirements Document  
Defines all functional and non‑functional requirements.

### **BTM001V125LTSE**  
Bidirectional Traceability Matrix  
Maps architecture → requirements → tests → evidence.

### **FULL SYSTEM TEST LETTER**  
A narrative explanation of how the test harness validates the system.

### **JACKHAMMER CYBER SAFETY STATEMENT**  
A high‑level safety and transparency declaration.

---

## Purpose of This Repository

This repository exists to provide:

- **Full transparency**  
- **Complete documentation**  
- **Deterministic reproducibility**  
- **Clear separation of authority**  
- **Safety‑critical engineering discipline**  
- **Hackathon‑ready disclosure**  

It is designed to be read by:

- Hackathon judges  
- Security auditors  
- Protocol researchers  
- Safety engineers  
- Compliance reviewers  

---

## Contact

**Author:**  
**John G. Brooks III**  
Founder, John G. Brooks Foundation

For questions related to this documentation package, please contact through the hackathon communication channels.

---

## Final Note

This repository is intentionally documentation‑only.  
The full codebase exists privately and is available for audit upon request under appropriate conditions.

This repo provides everything required for a complete, transparent, safety‑critical evaluation of Golden Governor Lite v1.25 and its integrated deterministic control system.
