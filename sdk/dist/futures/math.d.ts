/**
 * Exact decimal conversions between integer units and decimal strings (index
 * values, USDC amounts). The futures program's payoff and unit math lives in
 * `program.ts`.
 */
/** Formats integer units with `decimals` places, trimming trailing zeros (`102400000, 6` → `"102.4"`). */
export declare function formatUnits(units: bigint, decimals: number): string;
/** Parses a non-negative decimal string into integer units with `decimals` places (exact; rejects excess precision). */
export declare function parseUnits(value: string, decimals: number): bigint;
//# sourceMappingURL=math.d.ts.map