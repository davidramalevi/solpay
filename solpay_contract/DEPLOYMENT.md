# SolPay Clearinghouse - Deployment Guide

## Prerequisites

### 1. Install Rust
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
```

### 2. Install Solana CLI
```bash
# Windows (PowerShell as Administrator)
cmd /c "curl https://release.solana.com/v1.18.0/solana-install-init-x86_64-pc-windows-msvc.exe --output C:\solana-install-tmp\solana-install-init.exe --create-dirs"
C:\solana-install-tmp\solana-install-init.exe v1.18.0

# Add to PATH: C:\Users\<YOUR_USERNAME>\.local\share\solana\install\active_release\bin
```

### 3. Install Anchor CLI
```bash
cargo install --git https://github.com/coral-xyz/anchor avm --locked --force
avm install latest
avm use latest
```

## Deployment Steps

### 1. Configure Solana CLI for Devnet
```bash
solana config set --url https://api.devnet.solana.com
solana-keygen new  # Creates ~/.config/solana/id.json (your deployer wallet)
solana airdrop 2   # Get devnet SOL for deployment
```

### 2. Build the Program
```bash
cd solpay_contract
anchor build
```

This generates:
- `target/deploy/solpay_clearinghouse.so` (compiled program)
- `target/idl/solpay_clearinghouse.json` (interface definition)

### 3. Deploy to Devnet
```bash
anchor deploy
```

**Important**: The deployment will output a **Program ID**. You must update this in:
- `solpay_contract/programs/solpay_clearinghouse/src/lib.rs` → `declare_id!("...")`
- `solpay_contract/Anchor.toml` → `[programs.devnet]`
- `solpay_server/.env` → `PROGRAM_ID=...`

Then rebuild and redeploy:
```bash
anchor build
anchor deploy
```

### 4. Initialize the Clearinghouse
```bash
cd ../solpay_server
node src/scripts/init-clearinghouse.js
```

This will:
- Generate authority and fee collector wallets
- Fund them with devnet SOL
- Call the `initialize` instruction
- Create the pool PDA and token accounts
- Print the fee collector address to add to `.env`

### 5. Update Server Config
Copy the fee collector address from the init script output and add to `solpay_server/.env`:
```
FEE_COLLECTOR_ADDRESS=<address_from_init_script>
```

Restart the server:
```bash
npm run dev
```

## Current Status

**The backend server and Flutter app are fully functional WITHOUT deploying the smart contract.**

Currently, the system works in "direct mode":
- Wallet creation ✅ (real Solana wallets on devnet)
- Balance queries ✅ (real USDC balances from blockchain)
- Transaction history ✅ (real on-chain data)
- Solana Actions/Blinks ✅ (generates valid transaction URLs)

**What requires the smart contract:**
- Actual payment processing through the privacy pool
- Automated fee splitting on-chain
- Privacy guarantees (buyer/merchant anonymity)

## Testing Without Deployment

You can test the full app flow now:
1. Run backend: `cd solpay_server && npm run dev`
2. Run Flutter app: `cd solpay_app && flutter run`
3. Sign up → creates real Solana wallet on devnet
4. View dashboard → shows real balance from blockchain
5. Create payment → generates Blink URL (customers would need USDC to complete)

## Next Steps

1. Install Anchor CLI (see prerequisites above)
2. Deploy the smart contract to devnet
3. Initialize the clearinghouse
4. Test end-to-end payments with the privacy pool

## Troubleshooting

**"anchor: command not found"**
- Install Anchor CLI (see prerequisites)
- Ensure `~/.cargo/bin` is in your PATH

**"Insufficient funds for deployment"**
- Run `solana airdrop 2` to get more devnet SOL
- Check balance: `solana balance`

**"Program ID mismatch"**
- Update all three locations with the deployed program ID
- Rebuild and redeploy

## Resources

- Anchor Docs: https://www.anchor-lang.com/
- Solana Devnet Faucet: https://faucet.solana.com/
- Solana Explorer (Devnet): https://explorer.solana.com/?cluster=devnet
