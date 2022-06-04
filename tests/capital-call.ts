import * as fs from "fs";
import * as anchor from "@project-serum/anchor";
import {BN, Program, web3} from "@project-serum/anchor";
import {Keypair, PublicKey} from "@solana/web3.js";
import {CapitalCall} from "../target/types/capital_call";
import {AuthorityType, createAssociatedTokenAccountInstruction, createSetAuthorityInstruction} from "@solana/spl-token";

import * as chai from 'chai';
import {expect, use} from 'chai';
import chaiAsPromised from 'chai-as-promised';

chai.use(chaiAsPromised);

describe("capital-call", () => {
  // Configure the client to use the local cluster.
  anchor.setProvider(anchor.AnchorProvider.local(
    "http://localhost:8899",
    {commitment: "confirmed", preflightCommitment: "confirmed"}));

  const provider = anchor.getProvider();
  const program = anchor.workspace.CapitalCall as Program<CapitalCall>;
  const tokenProgram = anchor.Spl.token();

  const config = Keypair.generate();
  const authority = Keypair.generate();

  const mint = Keypair.fromSecretKey(new Uint8Array(JSON.parse(fs.readFileSync('keys/mint.json').toString())));
  const liquidityPool = Keypair.generate();
  const lpMint = Keypair.generate();

  const user1 = Keypair.generate();
  const user2 = Keypair.generate();

  // Values from Technical Specification
  const TSLiquidityPoolSize = 2435827;
    const TSCreditOutstanding = 7348028;
  const TSLpSupply = 9127492;

  let capitalCall1;

  async function getATA(owner: PublicKey, mint: PublicKey) {
    const [ata, _nonce] = await PublicKey.findProgramAddress(
      [owner.toBuffer(), anchor.utils.token.TOKEN_PROGRAM_ID.toBuffer(), mint.toBuffer()],
      anchor.utils.token.ASSOCIATED_PROGRAM_ID
    );
    return ata;
  }

  async function tokenBalance(token: PublicKey) {
    const account = await tokenProgram.account.token.fetch(token);
    return account.amount.toNumber();
  }

  before(async () => {
    // init mint for tokens
    await tokenProgram.methods.initializeMint(6, provider.wallet.publicKey, null)
      .accounts({
        mint: mint.publicKey,
        rent: web3.SYSVAR_RENT_PUBKEY,
      })
      .preInstructions([await tokenProgram.account.mint.createInstruction(mint)])
      .signers([mint]).rpc();

    // init account for liquidityPool
    await tokenProgram.methods.initializeAccount()
      .accounts({
        account: liquidityPool.publicKey,
        mint: mint.publicKey,
        authority: provider.wallet.publicKey,
        rent: web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([liquidityPool])
      .preInstructions([await tokenProgram.account.token.createInstruction(liquidityPool)])
      .rpc();

    // mint tokens for liquidityPool
    await tokenProgram.methods.mintTo(new BN(TSLiquidityPoolSize))
      .accounts({
        mint: mint.publicKey,
        to: liquidityPool.publicKey,
        authority: provider.wallet.publicKey,
      }).rpc();

    // init mint for lpMint
    await tokenProgram.methods.initializeMint(6, provider.wallet.publicKey, null)
      .accounts({
        mint: lpMint.publicKey,
        rent: web3.SYSVAR_RENT_PUBKEY,
      })
      .preInstructions([await tokenProgram.account.mint.createInstruction(lpMint)])
      .signers([lpMint]).rpc();

    // create and mint lp token circulation
    const lpTokenCirculation = Keypair.generate();
    await tokenProgram.methods.mintTo(new BN(TSLpSupply))
      .accounts({
        mint: lpMint.publicKey,
        to: lpTokenCirculation.publicKey,
        authority: provider.wallet.publicKey,
      }).preInstructions(
        [
          await tokenProgram.account.token.createInstruction(lpTokenCirculation),
          await tokenProgram.methods.initializeAccount()
            .accounts({
              account: lpTokenCirculation.publicKey,
              mint: lpMint.publicKey,
              authority: provider.wallet.publicKey,
              rent: web3.SYSVAR_RENT_PUBKEY,
            }).instruction(),
        ]
      )
      .signers([lpTokenCirculation]).rpc();


    // move lp mint authority to program
    const [lpMintAuthority] = await PublicKey.findProgramAddress(
      [anchor.utils.bytes.utf8.encode("lp_mint_authority"), config.publicKey.toBuffer()],
      program.programId
    );

    await provider.sendAndConfirm(new web3.Transaction()
      .add(
        createSetAuthorityInstruction(lpMint.publicKey, provider.wallet.publicKey, AuthorityType.MintTokens, lpMintAuthority)
      ));

    // Airdrop test users wallets
    await provider.connection.requestAirdrop(user1.publicKey, web3.LAMPORTS_PER_SOL);
    await provider.connection.requestAirdrop(user2.publicKey, web3.LAMPORTS_PER_SOL);

    // Init ata for tests users
    await provider.sendAndConfirm(
      new web3.Transaction()
        .add(
          createAssociatedTokenAccountInstruction(
            provider.wallet.publicKey,
            await getATA(user1.publicKey, mint.publicKey),
            user1.publicKey,
            mint.publicKey),
          createAssociatedTokenAccountInstruction(
            provider.wallet.publicKey,
            await getATA(user1.publicKey, lpMint.publicKey),
            user1.publicKey,
            lpMint.publicKey),
          createAssociatedTokenAccountInstruction(
            provider.wallet.publicKey,
            await getATA(user2.publicKey, mint.publicKey),
            user2.publicKey,
            mint.publicKey),
          createAssociatedTokenAccountInstruction(
            provider.wallet.publicKey,
            await getATA(user2.publicKey, lpMint.publicKey),
            user2.publicKey,
            lpMint.publicKey),
          createAssociatedTokenAccountInstruction(
            provider.wallet.publicKey,
            await getATA(provider.wallet.publicKey, mint.publicKey),
            provider.wallet.publicKey,
            mint.publicKey),
          createAssociatedTokenAccountInstruction(
            provider.wallet.publicKey,
            await getATA(provider.wallet.publicKey, lpMint.publicKey),
            provider.wallet.publicKey,
            lpMint.publicKey),
        ));
  });

  it("Initialize", async () => {
    await program.methods.initialize()
      .accounts({
        config: config.publicKey,
        lpMint: lpMint.publicKey,
        liquidityPool: liquidityPool.publicKey,
        payer: provider.wallet.publicKey,
        authority: authority.publicKey,
      }).signers([config])
      .rpc();

    const configAcc = await program.account.config.fetch(config.publicKey);
    expect(configAcc.authority).to.be.deep.equal(authority.publicKey);
    expect(configAcc.lpMint).to.be.deep.equal(lpMint.publicKey);
    expect(configAcc.liquidityPool).to.be.deep.equal(liquidityPool.publicKey);

    const [lpMintAuthority, _nonce] = await PublicKey.findProgramAddress(
      [anchor.utils.bytes.utf8.encode("lp_mint_authority"), config.publicKey.toBuffer()],
      program.programId
    );
    expect(configAcc.lpMintAuthority).to.be.deep.equal(lpMintAuthority);
  });

  it("Should create capital call", async() => {
    const blockTime = await provider.connection.getBlockTime(await provider.connection.getSlot());
    const duration = 10;

    const keys = await program.methods.createCapitalCall(
      new BN(blockTime + 1), // start_time
      new BN(duration), // duration
      new BN(2_000_000), // capacity - 2.0 Token
      new BN(TSCreditOutstanding) // credit outstanding
    ).accounts({
      config: config.publicKey,
      mint: mint.publicKey,
      lpMint: lpMint.publicKey,
      authority: authority.publicKey,
      payer: provider.wallet.publicKey,
    }).pubkeys();

    await program.methods.createCapitalCall(
      new BN(blockTime + 1), // start_time
      new BN(duration), // duration
      new BN(2_000_000), // capacity - 2.0 Token
      new BN(TSCreditOutstanding) // credit outstanding
    ).accounts({
      config: config.publicKey,
      mint: mint.publicKey,
      lpMint: lpMint.publicKey,
      authority: authority.publicKey,
      payer: provider.wallet.publicKey,
    }).signers([authority]).rpc();

    const capitalCallAcc = await program.account.capitalCall.fetch(keys.capitalCall);
    expect(capitalCallAcc.config).to.be.deep.equal(config.publicKey);
    expect(capitalCallAcc.vault).to.be.deep.equal(keys.vault);
    expect(capitalCallAcc.lpTokenPool).to.be.deep.equal(keys.lpTokenPool);
    expect(capitalCallAcc.startTime.toNumber()).to.be.equal(blockTime + 1);
    expect(capitalCallAcc.endTime.toNumber()).to.be.equal(blockTime + 1 + duration);
    expect(capitalCallAcc.capacity.toNumber()).to.be.equal(2_000_000);
    expect(capitalCallAcc.allocated.toNumber()).to.be.equal(0);
    expect(capitalCallAcc.redeemed.toNumber()).to.be.equal(0);
    expect(capitalCallAcc.tokenLiquidity.toNumber()).to.be.equal(0);
    expect(capitalCallAcc.lpSupply.toNumber()).to.be.equal(0);
    expect(capitalCallAcc.creditOutstanding.toNumber()).to.be.equal(TSCreditOutstanding);
    expect(capitalCallAcc.isLpMinted).to.be.false;

    capitalCall1 = keys.capitalCall;
    await new Promise(resolve => setTimeout(resolve, 2*1000));
  });

  it("Should deposit", async() => {
    // mint test users ata
    const ata1 = await getATA(user1.publicKey, mint.publicKey);
    await tokenProgram.methods.mintTo(new BN(1_000_000))
      .accounts({
        mint: mint.publicKey,
        to: ata1,
        authority: provider.wallet.publicKey,
      }).rpc();

    const ata2 = await getATA(user2.publicKey, mint.publicKey);
    await tokenProgram.methods.mintTo(new BN(2_000_000))
      .accounts({
        mint: mint.publicKey,
        to: ata2,
        authority: provider.wallet.publicKey,
      }).rpc();

    await program.methods.deposit(new BN(1_000_000))
      .accounts({
        capitalCall: capitalCall1,
        authority: user1.publicKey,
        source: ata1,
      }).signers([user1])
      .rpc();

    expect(await tokenBalance(ata1)).to.be.equal(0);
    let ccAcc = await program.account.capitalCall.fetch(capitalCall1);
    expect(ccAcc.allocated.toNumber()).to.be.equal(1_000_000);

    let [voucher] = await PublicKey.findProgramAddress(
      [anchor.utils.bytes.utf8.encode("voucher"), capitalCall1.toBuffer(), user1.publicKey.toBuffer()],
      program.programId,
    );
    let voucherAcc = await program.account.voucher.fetch(voucher);
    expect(voucherAcc.capitalCall).to.be.deep.equal(capitalCall1);
    expect(voucherAcc.authority).to.be.deep.equal(user1.publicKey);
    expect(voucherAcc.amount.toNumber()).to.be.deep.equal(1_000_000);

    await program.methods.deposit(new BN(2_000_000))
      .accounts({
        capitalCall: capitalCall1,
        authority: user2.publicKey,
        source: ata2,
      }).signers([user2])
      .rpc();

    expect(await tokenBalance(ata2)).to.be.equal(1_000_000);
    ccAcc = await program.account.capitalCall.fetch(capitalCall1);
    expect(ccAcc.allocated.toNumber()).to.be.equal(2_000_000);

    [voucher] = await PublicKey.findProgramAddress(
      [anchor.utils.bytes.utf8.encode("voucher"), capitalCall1.toBuffer(), user2.publicKey.toBuffer()],
      program.programId,
    );
    voucherAcc = await program.account.voucher.fetch(voucher);
    expect(voucherAcc.capitalCall).to.be.deep.equal(capitalCall1);
    expect(voucherAcc.authority).to.be.deep.equal(user2.publicKey);
    expect(voucherAcc.amount.toNumber()).to.be.deep.equal(1_000_000);
  })

  it("Should mint LP tokens", async() => {
    const keys = await program.methods.mintLpTokens()
      .accounts({
        config: config.publicKey,
        capitalCall: capitalCall1,
        liquidityPool: liquidityPool.publicKey,
        lpMint: lpMint.publicKey,
      }).pubkeys();

    await program.methods.mintLpTokens()
      .accounts({
        config: config.publicKey,
        capitalCall: capitalCall1,
        liquidityPool: liquidityPool.publicKey,
        lpMint: lpMint.publicKey,
      }).rpc();

    const ccAcc = await program.account.capitalCall.fetch(capitalCall1);
    expect(ccAcc.isLpMinted).to.be.true;
    expect(ccAcc.tokenLiquidity.toNumber()).to.be.equal(TSLiquidityPoolSize);
    expect(ccAcc.lpSupply.toNumber()).to.be.equal(TSLpSupply);

    const expected = 2_143_821; // 2_000_000 * ( TSLiquidityPoolSize + TSCreditOutstanding) / TSLpSupply
    expect(await tokenBalance(keys.lpTokenPool)).to.be.equal(expected);
  });

  it("Should claim lp tokens", async () => {
    const ata1 = await getATA(user1.publicKey, lpMint.publicKey);
    await program.methods.claim().accounts({
        capitalCall: capitalCall1,
        authority: user1.publicKey,
        destination: ata1,
      }).signers([user1])
      .rpc();

    const expectedBalance = 1_071_910 // 1_000_000 * ( TSLiquidityPoolSize + TSCreditOutstanding) / TSLpSupply
    expect(await tokenBalance(ata1)).to.be.equal(expectedBalance);

    let ccAcc = await program.account.capitalCall.fetch(capitalCall1);
    expect(ccAcc.redeemed.toNumber()).to.be.equal(1_000_000);

    const ata2 = await getATA(user2.publicKey, lpMint.publicKey);
    await program.methods.claim().accounts({
      capitalCall: capitalCall1,
      authority: user2.publicKey,
      destination: ata2,
    }).signers([user2])
      .rpc();

    expect(await tokenBalance(ata2)).to.be.equal(expectedBalance);
    ccAcc = await program.account.capitalCall.fetch(capitalCall1);
    expect(ccAcc.redeemed.toNumber()).to.be.equal(2_000_000);
  });

  it("Should close completed capital call", async () => {
    const ata = await getATA(provider.wallet.publicKey, mint.publicKey);

    const keys = await program.methods.close()
      .accounts({
        config: config.publicKey,
        capitalCall: capitalCall1,
        authority: authority.publicKey,
        receiver: provider.wallet.publicKey,
        lpMint: lpMint.publicKey,
        destination: ata,
      }).pubkeys();

    expect(await tokenBalance(keys.lpTokenPool)).to.be.equal(1);

    await program.methods.close()
      .accounts({
        config: config.publicKey,
        capitalCall: capitalCall1,
        authority: authority.publicKey,
        receiver: provider.wallet.publicKey,
        lpMint: lpMint.publicKey,
        destination: ata,
      }).signers([authority])
      .rpc();
  });

  async function blockTimeFromTx(txs) {
    const tx = await provider.connection.getTransaction(txs);
    return tx.blockTime;
  }

  it("Should create capital call and refund it", async() => {
    const blockTime = await provider.connection.getBlockTime(await provider.connection.getSlot());
    const duration = 3;

    const keys = await program.methods.createCapitalCall(
      new BN(blockTime + 1), // start_time
      new BN(duration), // duration
      new BN(2_000_000), // capacity - 2.0 Token
      new BN(TSCreditOutstanding) // credit outstanding
    ).accounts({
      config: config.publicKey,
      mint: mint.publicKey,
      lpMint: lpMint.publicKey,
      authority: authority.publicKey,
      payer: provider.wallet.publicKey,
    }).pubkeys();

    await program.methods.createCapitalCall(
      new BN(blockTime + 1), // start_time
      new BN(duration), // duration
      new BN(2_000_000), // capacity - 2.0 Token
      new BN(TSCreditOutstanding) // credit outstanding
    ).accounts({
      config: config.publicKey,
      mint: mint.publicKey,
      lpMint: lpMint.publicKey,
      authority: authority.publicKey,
      payer: provider.wallet.publicKey,
    }).signers([authority]).rpc();

    const capitalCall = keys.capitalCall;

    // wait until start_time
    await new Promise(resolve => setTimeout(resolve, 1500));

    const ata2 = await getATA(user2.publicKey, mint.publicKey);
    await program.methods.deposit(new BN(1_000_000))
      .accounts({
        capitalCall: capitalCall,
        authority: user2.publicKey,
        source: ata2,
      }).signers([user2])
      .rpc();

    expect(await tokenBalance(ata2)).to.be.equal(0);

    // wait until end_time
    await new Promise(resolve => setTimeout(resolve, 3500));

    await program.methods.refund()
      .accounts({
        capitalCall,
        authority: user2.publicKey,
        destination: ata2,
      }).signers([user2]).rpc();

    expect(await tokenBalance(ata2)).to.be.equal(1_000_000);
  });

  // TODO for tests (negative):
  // Should NOT init with invalid lp mint authority
  // Should NOT init with invalid liquidity_pool mint
  // Should NOT create capital call with invalid authority
  // Should NOT create capital call with invalid lpMint
  // Should NOT create capital call with invalid mint
  // Should NOT create capital call with start_time in past
  // Should NOT create capital call with zero duration
  // Should NOT create capital call with zero capacity
  // Should NOT deposit before start_time
  // Should NOT deposit after end_time
  // Should NOT deposit after capital fully funded
  // Should NOT deposit with zero amount
  // Should NOT refund until end_time
  // Should NOT refund if capital fully funded
  // Should NOT refund multiple times the same user
  // Should NOT refund user without voucher
  // Should NOT mint lp tokens with invalid lp mint authority
  // Should NOT mint lp tokens with zero lp supply
  // Should NOT fail mint lp tokens if capital call is not fully funded
  // Should NOT fail mint lp tokens if lp tokens already minted
  // Should NOT claim if lp tokens not minted
  // Should NOT claim multiple times the same user
  // Should NOT claim user without voucher
  // Should NOT close if not fully redeemed
  // Should close with extra tokens or lp tokens (funded with direct transfer)
});
