import test from 'node:test';
import assert from 'node:assert/strict';
import { PublicKey, TransactionInstruction } from '@solana/web3.js';
import { GovernanceGateBinding, findGovernanceGateAddress, findProgramDataAddress,
  LOCALNET_CONTROLLER_PROGRAM_ID, encodeGovernanceTail, decodeGovernanceTail, transactionErrorName } from '../dist/futures/index.js';

const controller = LOCALNET_CONTROLLER_PROGRAM_ID;
const target = new PublicKey(new Uint8Array(32).fill(25));
function fixture(program = target, epoch = 7n) {
  const [address, bump] = findGovernanceGateAddress(controller, program);
  const data = Buffer.alloc(192);
  data.write('AGVGAT01'); data[8] = 1; data[9] = bump; data[10] = 1;
  PublicKey.findProgramAddressSync([Buffer.from('ameba-governance-v3'), Buffer.from('council')], controller)[0].toBuffer().copy(data, 12);
  program.toBuffer().copy(data, 44);
  findProgramDataAddress(program)[0].toBuffer().copy(data, 76);
  data.writeBigUInt64LE(epoch, 108);
  return { address, data, binding: GovernanceGateBinding.fromAccount(controller, program, address, controller, data) };
}

test('governance gate authenticates identity and canonical active status', () => {
  const { address, data, binding } = fixture();
  assert.equal(binding.epoch, 7n);
  for (const at of [0, 8, 9, 10, 11, 12, 44, 76, 116, 148, 156, 190]) {
    const corrupt = Buffer.from(data); corrupt[at] ^= 1;
    assert.throws(() => GovernanceGateBinding.fromAccount(controller, target, address, controller, corrupt));
  }
  assert.throws(() => GovernanceGateBinding.fromAccount(controller, target, target, controller, data));
  assert.throws(() => GovernanceGateBinding.fromAccount(controller, target, address, target, data));
  assert.throws(() => GovernanceGateBinding.fromAccount(controller, target, address, controller, data.subarray(0, 191)));
  assert.equal(encodeGovernanceTail(7n).toString('hex'), '41475631010000000700000000000000');
  assert.throws(() => encodeGovernanceTail(-1n));
  const zero = Buffer.from(data); zero.writeBigUInt64LE(0n, 108);
  assert.throws(() => GovernanceGateBinding.fromAccount(controller, target, address, controller, zero));
});

test('a bound instruction keeps its raw data and accounts and ends in the gate', () => {
  const { binding } = fixture();
  const raw = new TransactionInstruction({ programId: target, data: Buffer.from([3, 2]),
    keys: Array.from({ length: 3 }, (_, i) => ({ pubkey: new PublicKey(new Uint8Array(32).fill(i + 40)), isSigner: false, isWritable: false })) });
  const bound = binding.bind(raw);
  assert.equal(bound.data.toString('hex'), '0302' + encodeGovernanceTail(7n).toString('hex'));
  assert.deepEqual(decodeGovernanceTail(bound.data), { payload: Uint8Array.from([3, 2]), epoch: 7n });
  assert.deepEqual(bound.keys.at(-1), { pubkey: binding.address, isSigner: false, isWritable: false });
  assert.equal(raw.data.length, 2); assert.equal(raw.keys.length, 3);
  assert.throws(() => binding.bind(bound));
  assert.throws(() => binding.bind(new TransactionInstruction({ programId: new PublicKey(new Uint8Array(32).fill(26)), keys: [], data: Buffer.from([0]) })));
});

test('shared verifier errors have distinct governance names', () => {
  assert.equal(transactionErrorName(3108), 'GovernanceGateFrozen');
  assert.equal(transactionErrorName(3109), 'GovernanceGateEpochMismatch');
  assert.equal(transactionErrorName(3110), null);
});
