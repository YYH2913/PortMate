/** Nearest-rank percentiles. Retain the sample count and maximum with P99. */
export function latencySummary(values) {
  if (!values.length || values.some(value => !Number.isFinite(value) || value < 0)) {
    throw new Error("Latency samples must be nonempty, finite and nonnegative");
  }
  const sorted = [...values].sort((left, right) => left - right);
  const percentile = fraction => sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)];
  const round = value => Math.round(value * 100) / 100;
  return {
    samples: sorted.length,
    p50: round(percentile(.5)), p95: round(percentile(.95)),
    p99: round(percentile(.99)), max: round(sorted.at(-1)),
    over50: sorted.filter(value => value > 50).length,
    over100: sorted.filter(value => value > 100).length,
    over250: sorted.filter(value => value > 250).length,
  };
}
