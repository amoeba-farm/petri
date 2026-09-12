// Read-only carry-forward projection from canonical SDK-derived native accounts.
import {createRequire} from 'node:module';
import {oracleContext} from './sdk-oracle.mjs';
const require=createRequire(new URL('./sdk/package.json',import.meta.url));
const {PublicKey,SystemProgram}=require('@solana/web3.js');
const carry=await import('./sdk/node_modules/@amoeba/spread-release-tools/dist/amoebaOracleCarryForward.js');
const native=await import('./sdk/node_modules/@amoeba/spread-release-tools/dist/amoebaOracleDlmmInstructions.js');
const fail=message=>{throw new Error(message);};
const plain=v=>typeof v==='bigint'?v.toString():Buffer.isBuffer(v)?v.toString('hex'):v instanceof PublicKey?v.toBase58():Array.isArray(v)?v.map(plain):v&&typeof v==='object'?Object.fromEntries(Object.entries(v).map(([k,value])=>[k,plain(value)])):v;
export async function readCarry({adapter,connection,request}) {
  if(Object.keys(request).some(k=>!['marketId','expiryId','sourceId'].includes(k)))fail('Unknown carry read field');
  if(request.sourceId!==undefined&&!/^[0-9a-f]{64}$/.test(request.sourceId))fail('Source ID must be 64 lowercase hex characters');
  const c=await oracleContext({adapter,connection,request});
  const periodAddress=carry.deriveOracleCarryAddress('period',c.oracleMonthAddress,c.programId);
  const registryAddress=carry.deriveOracleCarryRegistryAddress(c.market.underlyingId,c.programId);
  const source=request.sourceId===undefined?null:native.deriveOracleSourcePda({oracleMonthPda:c.oracleMonthAddress,sourceId:Buffer.from(request.sourceId,'hex'),programId:c.programId});
  const sourceAddress=source?carry.deriveOracleCarryAddress('source',source,c.programId):null;
  const journalAddress=source?carry.deriveOracleCarryAddress('journal',source,c.programId):null;
  const addresses=[periodAddress,registryAddress,...(source?[sourceAddress,journalAddress]:[])];
  const snapshot=await connection.getMultipleAccountsInfoAndContext(addresses,{commitment:'finalized',minContextSlot:c.observedSlot});
  const decode=(info,address,fn)=>!info||(!info.executable&&info.owner.equals(SystemProgram.programId)&&info.data.length===0)?null:fn({...info,address,programId:c.programId});
  const period=decode(snapshot.value[0],periodAddress,carry.decodeOracleCarryPeriodV1);
  const registry=decode(snapshot.value[1],registryAddress,carry.decodeOracleCarryRegistryV1);
  const state=source?decode(snapshot.value[2],sourceAddress,carry.decodeOracleCarrySourceV1):null;
  const journal=source?decode(snapshot.value[3],journalAddress,carry.decodeOracleKnowledgeJournalV1):null;
  if(period&&(!period.month.equals(c.oracleMonthAddress)||!period.underlying.equals(c.market.underlyingId)||period.expiry!==c.market.expiryTs))fail('Carry period does not match this exact series');
  if(registry&&!registry.underlying.equals(c.market.underlyingId))fail('Carry registry underlying mismatch');
  if(state&&(!state.month.equals(c.oracleMonthAddress)||!state.source.equals(source)))fail('Carry source belongs to another period');
  if(journal&&!journal.source.equals(source))fail('Carry journal belongs to another source');
  let checkpoint=null,observedSlot=snapshot.context.slot;
  if(state&&!state.selectedCheckpoint.equals(PublicKey.default)) {
    // Resolve the checkpoint chosen on chain, never a caller-selected value.
    const fresh=await connection.getMultipleAccountsInfoAndContext([sourceAddress,state.selectedCheckpoint],{commitment:'finalized',minContextSlot:observedSlot});
    if(!fresh.value[0]||!fresh.value[0].owner.equals(snapshot.value[2].owner)||fresh.value[0].executable!==snapshot.value[2].executable||!fresh.value[0].data.equals(snapshot.value[2].data))fail('Carry selection changed during the read; refresh');
    checkpoint=decode(fresh.value[1],state.selectedCheckpoint,carry.decodeOracleAcceptedCheckpointV1);
    if(!checkpoint||!checkpoint.source.equals(state.parentSource)||!checkpoint.month.equals(state.parentMonth)||checkpoint.observedAt!==state.observedAt||checkpoint.value!==state.value||checkpoint.sequence!==state.selectedSequence)fail('Selected carry checkpoint provenance mismatch');
    observedSlot=fresh.context.slot;
  }
  const status=state?carry.OracleCarrySourceStatusV1[state.status]:null;
  const next=state?.status===carry.OracleCarrySourceStatusV1.Imported?'BeginSelection':state?.status===carry.OracleCarrySourceStatusV1.Selecting?(state.remaining>0?'ScanCheckpoint':'FreezeOpening'):null;
  return plain({marketId:request.marketId,expiryId:request.expiryId,sourceId:request.sourceId??null,observedSlot:String(observedSlot),commitment:'finalized',period,registry,source:state,journal,checkpoint,
    status,openingProvenance:status==='FrozenCarry'?'carried':status==='FreshOpening'?'fresh':null,
    originalObservedAt:state&&state.observedAt>0n?state.observedAt:null,acceptedAt:state&&state.acceptedAt>0n?state.acceptedAt:null,
    progress:period?{importsCompleted:period.nextImport,importsRequired:period.expectedImports,checkpointsRemaining:state?.remaining??null}:null,
    maintenance:{candidateStage:next,admission:'not_evaluated',publicPrepareAvailable:false,reason:'The current hosted public request union has no carry preparation. Native lifecycle plans are role-scoped; this read does not authorize a transaction.'},
    coverage:{exactSeries:true,sourceSelected:source!==null,checkpointHistoryExhaustive:false},willSign:false,willSubmit:false});
}
