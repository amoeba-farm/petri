// Private, signer-free adapter. Only Rust supplies input; no CLI/eval/sign/submit mode.
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { isDeepStrictEqual } from 'node:util';
const require = createRequire(new URL('./sdk/package.json',import.meta.url));
const { Connection, PublicKey, TransactionInstruction, VersionedTransaction, TransactionMessage, AddressLookupTableAccount, SYSVAR_CLOCK_PUBKEY }=require('@solana/web3.js');
const sdk=await import('./sdk/dist/protocol/index.js');
const {rebuildOracleLogical}=await import('./sdk-oracle.mjs');
const {readCarry}=await import('./sdk-carry.mjs');
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const fail=message=>{throw new Error(message);};
const canonical=v=>Array.isArray(v)?v.map(canonical):v&&typeof v==='object'?Object.fromEntries(Object.keys(v).sort().map(k=>[k,canonical(v[k])])):v;
const same=(a,b,label)=>{if(!isDeepStrictEqual(canonical(a),canonical(b)))fail(`${label} differs from the exact requested SDK operation`);};
let size=0;const parts=[];
for await(const chunk of process.stdin){size+=chunk.length;if(size>8*1024*1024)fail('Worker input too large');parts.push(chunk);}
try {
  const input=JSON.parse(Buffer.concat(parts).toString('utf8'));
  if(input.schemaVersion!==1||!['collateral','oracle','liquidity','carry'].includes(input.family))fail('Unsupported SDK adapter request');
  const owner=new PublicKey(input.owner);
  if(owner.toBase58()!==input.owner)fail('Owner is not canonical');
  const rpcUrl=new URL('/rpc',input.backend).href;
  const allowed=new Set(['getAccountInfo','getMultipleAccounts','getSlot','getBlockTime','getGenesisHash','getLatestBlockhash','getBlockHeight','isBlockhashValid','getCompressedAccountV2','getValidityProofV2']);
  const transportFetch=globalThis.fetch.bind(globalThis);
  const guardedFetch=async(url,options)=>{
    if(String(url)!==rpcUrl||options?.method!=='POST')fail('Unexpected SDK transport');
    const body=JSON.parse(options.body);if(!allowed.has(body.method))fail('SDK worker transport is read-only');
    const response=await transportFetch(url,{...options,redirect:'error',signal:AbortSignal.timeout(20000)});
    const parts=[];let size=0;
    for await(const chunk of response.body){size+=chunk.length;if(size>48*1024*1024)fail('SDK read exceeds size bound');parts.push(chunk);}
    return new Response(Buffer.concat(parts),{status:response.status,headers:response.headers});
  };
  // Light's Photon transport uses global fetch, not ConnectionConfig.fetch.
  // This worker is a single-request isolated process; guard both call paths.
  globalThis.fetch=guardedFetch;
  const connection=new Connection(rpcUrl,{commitment:'finalized',disableRetryOnRateLimit:true,fetch:guardedFetch});
  const adapter=sdk.createCurrentSdkAdapter({connection,programId:sdk.AMOEBA_SPREAD_PROGRAM_ID,
    namespace:sdk.CURRENT_NAMESPACE,cluster:sdk.CURRENT_PROTOCOL_CLUSTER,
    releaseTag:sdk.CURRENT_PROTOCOL_RELEASE,releaseCommit:sdk.CURRENT_PROTOCOL_SOURCE_COMMIT});
  if(input.family==='carry') {
    const carry=await readCarry({adapter,connection,request:input.request});
    process.stdout.write(JSON.stringify({ok:true,sdkCommit:'a21b324a7a64da87046c7650355b80ea20c47540',owner:input.owner,carry}));
    process.exit(0);
  }
  const data=input.prepared.data??input.prepared;
  let plan;let prepared;let observation=null;
  if(input.family==='collateral') {
    prepared=data.action;
    const request={action:input.request.actionType,ownerPubkey:input.owner,...(input.request.amount===undefined?{}:{amount:BigInt(input.request.amount)})};
    const context=await adapter.prepareCurrentPositionActionContext({request});
    plan=await adapter.buildCurrentPositionAction({request,context});
    adapter.validateCurrentOperationPlan(plan);
    plan=sdk.currentOperationPlanToJson(plan);
    same(prepared.instructions,plan.instructions,'Collateral instructions');
    same(prepared.writeSet,plan.writeSet,'Collateral write set');
    same(prepared.signerRoles,plan.signerRoles,'Collateral signer roles');
    if(prepared.ownerPubkey!==input.owner||prepared.actionType!==request.action)fail('Collateral review identity changed');
    observation=sdk.validateCurrentFinalizedObservation(prepared.observation);
  } else if(input.family==='oracle') {
    prepared=data.draft;
    sdk.validateCurrentOracleActionRequest(input.request);
    const request={...input.request};delete request.secretSaltHex;
    same(prepared.request,request,'Oracle request');
    sdk.validateCurrentOperationPlanJson(canonical(prepared));
    plan=prepared;
    // Portable validation proves structure, not that a remote instruction spends
    // the user's requested amount. Rebuild semantics through the SDK as well.
    if(plan.compressedTransport!==null) {
      const local=await rebuildOracleLogical({adapter,connection,request:input.request,rpcUrl,guardedFetch,remote:plan});
      const remote=plan.logicalInstruction;
      same(local,{programId:remote.programId,dataBase64:remote.dataBase64,accounts:remote.accounts},'Oracle requested action');
    } else {
      const local=await adapter.buildCurrentOracleDraft({request:input.request});
      adapter.validateCurrentOperationPlan(local);
      same(local.logicalInstruction,plan.logicalInstruction,'Oracle requested action');
    }
  } else {
    prepared=data;
    const request=sdk.canonicalCurrentLiquidityRequest(input.request);
    same(prepared.request,request,'Liquidity request');
    const market=await adapter.readCurrentMarketAccount({marketId:request.marketId,expiryId:request.expiryId});
    const poolRead=await adapter.readCurrentAmoebaDlmmPool({marketId:request.marketId,expiryId:request.expiryId});
    if(!market.found||!market.market||!poolRead.pool)fail('Current liquidity market or pool is unavailable');
    observation=sdk.validateCurrentFinalizedObservation(prepared.currentObservation);
    const descriptor=sdk.currentLiquidityBuilderInput({request,market:market.address,optionMint:poolRead.pool.optionMint,
      quoteMint:poolRead.pool.quoteMint,programId:new PublicKey(sdk.AMOEBA_SPREAD_PROGRAM_ID),pageIndices:prepared.liquidity.pageIndices});
    const materialized=await adapter.buildCurrentGovernedInstruction(descriptor);
    if(materialized.instructions.length!==1)fail('Liquidity instruction count changed');
    const rebuilt={request,market:market.address.toBase58(),optionMint:poolRead.pool.optionMint.toBase58(),quoteMint:poolRead.pool.quoteMint.toBase58(),
      pageIndices:prepared.liquidity.pageIndices,currentObservation:observation,leanAdmissionDigest:prepared.leanAdmissionDigest,
      instruction:materialized.instructions[0],recentBlockhash:prepared.transaction.recentBlockhash};
    // Strip only the documented HTTP projection fields, not operation-plan fields.
    plan={...prepared};for(const key of ['marketId','expiryId','ownerPubkey','action','positionNonce','protocol'])delete plan[key];
    sdk.validateCurrentLiquidityOperationPlan(plan,rebuilt);
  }
  if(!/^[0-9a-f]{64}$/.test(prepared.operationId)||!/^[0-9a-f]{64}$/.test(prepared.preparedPlanDigest))fail('Operation identity is invalid');
  if(prepared.setupTransactions.length||plan.setupInstructionBatches.length)fail('This action requires a separately finalized setup stage');
  const instructions=plan.instructions.map(ix=>new TransactionInstruction({programId:new PublicKey(ix.programId),
    data:Buffer.from(ix.dataBase64,'base64'),keys:ix.accounts.map(a=>({pubkey:new PublicKey(a.pubkey),isSigner:a.isSigner,isWritable:a.isWritable}))}));
  const bytes=Buffer.from(prepared.transaction.serializedTransactionBase64,'base64');
  const tx=VersionedTransaction.deserialize(bytes);
  let tables=[];
  if(tx.message.addressTableLookups.length) {
    if(input.family!=='oracle'||!plan.transactionLookupTable)fail('Unexpected address lookup table');
    const witness=plan.transactionLookupTable;
    const info=await connection.getAccountInfo(new PublicKey(witness.address),'finalized');
    if(!info||sha(info.data)!==witness.accountDataSha256)fail('Lookup table changed');
    tables=[new AddressLookupTableAccount({key:new PublicKey(witness.address),state:AddressLookupTableAccount.deserialize(info.data)})];
  }
  const ixView=ix=>({programId:ix.programId.toBase58(),dataBase64:ix.data.toString('base64'),accounts:ix.keys.map(k=>({pubkey:k.pubkey.toBase58(),isSigner:k.isSigner,isWritable:k.isWritable}))});
  const message=new TransactionMessage({payerKey:owner,recentBlockhash:tx.message.recentBlockhash,instructions});
  const rebuiltMessage=tx.message.version==='legacy'?message.compileToLegacyMessage():message.compileToV0Message(tables);
  if(!Buffer.from(rebuiltMessage.serialize()).equals(Buffer.from(tx.message.serialize())))fail('Unsigned transaction differs from SDK instructions');
  if(tx.signatures.length!==1||tx.signatures.some(s=>s.some(b=>b!==0)))fail('Transaction does not have the exact unsigned owner');
  if(bytes.length>1232)fail('Transaction exceeds packet limit');
  if(!(await connection.isBlockhashValid(tx.message.recentBlockhash,{commitment:'finalized'})).value)fail('Review expired; prepare it again');
  const addresses=[...new Set(instructions.flatMap(ix=>[ix.programId,...ix.keys.map(k=>k.pubkey)]).map(k=>k.toBase58()))];
  if(addresses.length>100)fail('Operation account bound exceeded');
  const snapshot=await connection.getMultipleAccountsInfoAndContext(addresses.map(k=>new PublicKey(k)),{commitment:'finalized',...(observation?{minContextSlot:Number(observation.observedAtSlot)}:{})});
  const accounts=snapshot.value.map((account,i)=>({address:addresses[i],owner:account?.owner.toBase58()??null,executable:account?.executable??false,
    dataSha256:account?sha(account.data):null,dataLength:account?.data.length??0}));
  const clock=SYSVAR_CLOCK_PUBKEY.toBase58();
  // Clock data advances normally. The fresh SDK reconstruction checks phase/time;
  // its account identity still must match. Never exempt business-state accounts.
  if(observation)for(const old of observation.orderedAccounts){const now=accounts.find(a=>a.address===old.address);if(now&&(now.owner!==old.owner||(now.address!==clock&&now.dataSha256!==old.dataSha256)))fail('Prepared account state changed; prepare again');}
  const stable=rows=>rows.map(row=>row.address===clock?{...row,dataSha256:null}:row);
  if(input.expectedAccounts)same(stable(accounts),stable(input.expectedAccounts),'Finalized account snapshot');
  process.stdout.write(JSON.stringify({ok:true,sdkCommit:'a21b324a7a64da87046c7650355b80ea20c47540',operationId:prepared.operationId,
    preparedPlanDigest:prepared.preparedPlanDigest,owner:input.owner,serializedTransactionBase64:bytes.toString('base64'),
    messageSha256:sha(tx.message.serialize()),instructions:instructions.map(ixView),accounts,observedSlot:String(snapshot.context.slot)}));
} catch(error) { process.stdout.write(JSON.stringify({ok:false,error:{code:typeof error?.code==='string'?error.code:'SDK_OPERATION_REJECTED',message:String(error?.message??'SDK rejected operation').slice(0,2000)}}));process.exitCode=1; }
