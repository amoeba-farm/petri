/**
 * Exact decimal conversions between integer units and decimal strings (index
 * values, USDC amounts). The futures program's payoff and unit math lives in
 * `program.ts`.
 */

function pow10(decimals: number): bigint {
  if (!Number.isInteger(decimals) || decimals < 0 || decimals > 38) throw new RangeError("decimals must be 0..38");
  return 10n ** BigInt(decimals);
}

/** Formats integer units with `decimals` places, trimming trailing zeros (`102400000, 6` → `"102.4"`). */
export function formatUnits(units: bigint, decimals: number): string {
  const scale = pow10(decimals);
  const negative = units < 0n;
  const abs = negative ? -units : units;
  const whole = abs / scale;
  const frac = (abs % scale).toString().padStart(decimals, "0").replace(/0+$/u, "");
  return `${negative ? "-" : ""}${whole}${frac ? `.${frac}` : ""}`;
}

/** Parses a non-negative decimal string into integer units with `decimals` places (exact; rejects excess precision). */
export function parseUnits(value: string, decimals: number): bigint {
  const m = /^(\d+)(?:\.(\d*))?$/u.exec(value.trim());
  if (!m) throw new RangeError(`invalid decimal ${JSON.stringify(value)}`);
  const frac = (m[2] ?? "").replace(/0+$/u, "");
  if (frac.length > decimals) throw new RangeError(`more than ${decimals} decimal places`);
  return BigInt(m[1]!) * pow10(decimals) + BigInt(frac.padEnd(decimals, "0") || "0");
}
