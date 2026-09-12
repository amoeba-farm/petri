// CLI-owned gateway composition; all address derivation, account/leaf codecs,
// business planning and proof verification remain in the immutable SDK packages.
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { readFileSync } from 'node:fs';
const require=createRequire(new URL('./sdk/package.json',import.meta.url));
const native=await import(new URL('./sdk/node_modules/@amoeba/spread-release-tools/dist/amoebaOracleDlmmInstructions.js',import.meta.url));
const compressed=await import(new URL('./sdk/node_modules/@amoeba/spread-release-tools/dist/compressedState.js',import.meta.url));
const sdk=await import('./sdk/dist/protocol/index.js');
const governed=await import('./sdk/dist/protocol/current-governed-write-internal.js');
const {currentGovernedWriteReleaseV1}=await import('./sdk/dist/protocol/current-governed-release-internal.js');
const {prepareCurrentOracleAction}=await import('./sdk/dist/protocol/current-oracle-planner.js');
const {readCurrentVerifiedCompressedStateEvidence}=await import('./sdk/dist/protocol/current-photon-verified.js');
const {createCurrentCompressedEvidenceVerifier}=await import('./sdk/dist/protocol/current-compressed-evidence-verifier.js');
const light=require('@lightprotocol/stateless.js');
const {PublicKey,SystemProgram}=require('@solana/web3.js');
const hash=(...parts)=>{const h=createHash('sha256');for(const part of parts)h.update(part);return h.digest('hex');};
const fail=message=>{throw new Error(message);};
const hex=bn=>{if(bn.isNeg()||bn.byteLength()>32)fail('Invalid Light field');return bn.toArrayLike(Buffer,'be',32).toString('hex');};
const topology=Object.freeze({addressTree:new PublicKey(light.batchAddressTree).toBase58(),addressQueue:new PublicKey(light.batchAddressTree).toBase58(),
  stateTrees:Object.freeze([1,2,3,4,5].map(i=>Object.freeze({stateTree:new PublicKey(light[`batchMerkleTree${i}`]).toBase58(),queue:new PublicKey(light[`batchQueue${i}`]).toBase58(),cpiContext:new PublicKey(light[`batchCpiContext${i}`]).toBase58()})))});

export async function oracleContext({adapter,connection,request,bound=false}) {
  const programId=bound?(await governed.prepareBoundCurrentGovernedWriteV1({rpc:connection,minimumContextSlot:await connection.getSlot('finalized'),release:currentGovernedWriteReleaseV1()})).governedProgramId:new PublicKey(sdk.AMOEBA_SPREAD_PROGRAM_ID);
  const read=await adapter.readCurrentMarketAccount({marketId:request.marketId,expiryId:request.expiryId});
  if(!read.found||!read.market)fail('Current Oracle market is unavailable');
  const market=read.market,marketAddress=read.address;
  const oracleMonthAddress=native.deriveOracleMonthPda({marketPda:marketAddress,expiryTs:market.expiryTs,programId});
  const vaultConfigAddress=native.deriveVaultConfigPda(programId);
  const snapshot=await connection.getMultipleAccountsInfoAndContext([oracleMonthAddress,vaultConfigAddress],{commitment:'finalized'});
  const decode=(info,address,fn)=>{
    if(!info||info.executable||!info.owner.equals(programId))fail('Current Oracle account identity is unavailable');
    return fn({address,data:info.data,owner:info.owner,executable:info.executable,namespace:sdk.CURRENT_NAMESPACE,programId,expiryTs:market.expiryTs});
  };
  const oracleMonth=decode(snapshot.value[0],oracleMonthAddress,sdk.decodeCurrentOracleMonthAccount);
  const vaultConfig=decode(snapshot.value[1],vaultConfigAddress,sdk.decodeCurrentVaultConfigAccount);
  return {connection,commitment:'finalized',programId,marketAddress,market,oracleMonthAddress,oracleMonth,vaultConfigAddress,vaultConfig,observedSlot:snapshot.context.slot};
}

export async function rebuildOracleLogical({adapter,connection,request,rpcUrl,guardedFetch,remote}) {
  const context=await oracleContext({adapter,connection,request,bound:true});
  const nodeFile=process.platform==='win32'?'compressed-verifier.exe':'compressed-verifier';
  const receipt=JSON.parse(readFileSync(new URL('./compressed-verifier.json',import.meta.url),'utf8'));
  const verifier=createCurrentCompressedEvidenceVerifier({executablePath:fileURLToPath(new URL(`./${nodeFile}`,import.meta.url)),executableSha256:receipt.sha256,timeoutMs:20000});
  // The hosted /rpc endpoint is an identity-gated read-only Solana/Photon gateway.
  // Do not mislabel it as a private Helius endpoint to defeat the SDK's private
  // provider factory. Compose its real proof transport explicitly instead.
  const rpc=light.createRpc(rpcUrl,rpcUrl,rpcUrl,{commitment:'finalized',fetch:guardedFetch});
  const cache=new Map();
  const providerOriginSha256=hash(new URL(rpcUrl).origin);
  const readProof=async(hashes,addresses)=>{
    const response=await rpc.getValidityProofAndRpcContext(hashes,addresses);
    if(addresses.length===0)return response;
    // Pinned Light V2 puts the requested new address in `leaves`. The SDK's
    // native-read composition represents that slot as an absent stored leaf.
    // Check the actual address before translating this representation. Root,
    // proof, tree and requested address stay intact; the native verifier derives
    // and verifies that same address independently, never a substituted zero.
    if(hashes.length!==0||addresses.length!==1||response.value.leaves.length!==1||!response.value.leaves[0].eq(addresses[0].address))fail('Nonmembership proof returned a different requested address');
    return {...response,value:{...response.value,leaves:[light.createBN254(0)]}};
  };
  const photonConnection={topology,async observeCurrentCompressedState({canonicalPda,domain}) {
    const key=`${canonicalPda.toBase58()}:${domain}`;
    if(cache.has(key))return cache.get(key);
    if(canonicalPda.equals(SystemProgram.programId))fail('Invalid Oracle business PDA');
    const address=compressed.deriveCompressedStateAddressV1(context.programId,new PublicKey(topology.addressTree),domain,canonicalPda);
    const account=await rpc.getCompressedAccount(light.createBN254(address.toBytes()));
    // Authenticate the very same raw leaf (or absence), including native root/
    // queue verification. This read evidence is NOT passed as a transaction
    // transport observation. The execution witness below uses an actual proof.
    await readCurrentVerifiedCompressedStateEvidence({stateConnection:connection,providerOriginSha256,topology,verifier,canonicalPda,domain,
      minimumContextSlot:context.observedSlot,getCompressedAccount:async()=>account,getValidityProof:readProof});
    if(account===null){cache.set(key,null);return null;}
    const leaf=compressed.decodeCompressedAmebaStateLeaf(account);
    const response=await rpc.getValidityProofAndRpcContext([{hash:account.hash,tree:account.treeInfo.tree,queue:account.treeInfo.queue}],[]);
    const value=response.value,proof=value.compressedProof,tree=value.treeInfos[0];
    const finalized=await connection.getSlot('finalized');
    if(!Number.isSafeInteger(response.context.slot)||response.context.slot<1||finalized<response.context.slot||!proof||
      [value.roots,value.rootIndices,value.leaves,value.leafIndices,value.treeInfos,value.proveByIndices].some(a=>a.length!==1)||
      value.roots[0].isZero()||!Number.isInteger(value.rootIndices[0])||value.rootIndices[0]<0||value.rootIndices[0]>65535||
      value.leafIndices[0]!==account.leafIndex||value.proveByIndices[0]!==account.proveByIndex||!value.leaves[0].eq(account.hash)||
      !tree.tree.equals(account.treeInfo.tree)||!tree.queue.equals(account.treeInfo.queue)||tree.treeType!==light.TreeType.StateV2||!tree.cpiContext?.equals(account.treeInfo.cpiContext))fail('Oracle execution proof does not bind the current leaf');
    const parts=[['a',32],['b',64],['c',32]].map(([key,length])=>{const bytes=proof[key];if(bytes.length!==length||bytes.some(b=>!Number.isInteger(b)||b<0||b>255))fail('Invalid execution proof bytes');return Buffer.from(bytes);});
    const proofBytes=Buffer.concat(parts);
    const facts={evidenceTrust:'authorized_photon_provider',proofVerification:'onchain_at_execution',finalizedRootVerified:false,
      proofBase64:proofBytes.toString('base64'),proofStatementJson:JSON.stringify({kind:'membership',requestedHash:hex(account.hash),compressedAddress:address.toBase58(),
        roots:value.roots.map(hex),rootIndices:value.rootIndices,leaves:value.leaves.map(hex),leafIndices:value.leafIndices,proveByIndices:value.proveByIndices,
        trees:value.treeInfos.map(t=>({tree:t.tree.toBase58(),queue:t.queue.toBase58(),treeType:t.treeType})),contextSlot:response.context.slot}),
      providerOriginSha256,canonicalPda:canonicalPda.toBase58(),compressedAddress:address.toBase58(),domain,revision:leaf.revision.toString(),dataSha256:hash(leaf.data),
      stateTree:tree.tree.toBase58(),queue:tree.queue.toBase58(),cpiContext:tree.cpiContext.toBase58(),leafHash:hex(account.hash),leafIndex:String(account.leafIndex),root:hex(value.roots[0]),rootIndex:String(value.rootIndices[0]),
      proveByIndex:account.proveByIndex,proofContextSlot:String(response.context.slot),stateFinalizedSlot:String(finalized),proofSha256:hash(proofBytes)};
    const observation={canonicalPda,compressedAddress:address,leaf,witness:{...facts,witnessDigest:hash('ameba-spread-v2/current-compressed-state-witness-v1\0',JSON.stringify(facts))}};
    cache.set(key,observation);return observation;
  }};
  const local=await prepareCurrentOracleAction({...context,request,photonConnection,compressedObservations:[]});
  // SDK portable validation binds the remote envelope to the logical instruction.
  // Independently reconstruct that instruction from the current business state;
  // never create a second outer transaction with a new proof or blockhash.
  const instruction=governed.semanticCurrentSpreadInstructionV1(local.instruction);
  if(remote.logicalInstruction.tag!==local.tag||remote.actionType!==local.actionType)fail('Oracle logical action changed');
  return {programId:instruction.programId.toBase58(),dataBase64:instruction.data.toString('base64'),accounts:instruction.keys.map(k=>({pubkey:k.pubkey.toBase58(),isSigner:k.isSigner,isWritable:k.isWritable}))};
}
