/**
 * JSON parsing that keeps every integer exact.
 *
 * `JSON.parse` converts a number to a double *before* any reviver runs, so by
 * the time you could intervene `9007199254740993` is already
 * `9007199254740992`. The reviver's source-text argument is the only way to see
 * the digits the server actually sent.
 *
 * Every integer becomes a `bigint`. Uniformly, rather than for a list of
 * money-carrying field names: a list has to be maintained, and the day someone
 * adds a field and forgets to list it, a balance rounds silently. Uniform is
 * also the ergonomic choice here — every number this API returns is an integer,
 * and every calculation over them (`10n ** base_decimals`, `notional * bps /
 * 10_000n`) is integer arithmetic.
 */
export function parseExact(text: string): unknown {
  return JSON.parse(text, function (_key: string, value: unknown, context?: { source?: string }) {
    if (typeof value !== "number") return value;

    const source = context?.source;
    if (source === undefined) {
      // Refuse rather than hand back a number that may already have been
      // rounded. Silently wrong money is the one failure worth crashing over.
      throw new Error(
        "this runtime does not expose JSON source text to the reviver, so " +
          "64-bit integers cannot be read exactly; a recent browser is required",
      );
    }

    return /^-?\d+$/.test(source) ? BigInt(source) : value;
  });
}

/**
 * Serialise, emitting every `bigint` as an exact JSON number.
 *
 * `JSON.stringify` throws on a bigint, and the obvious workaround — a replacer
 * returning `Number(value)` — reintroduces the very rounding the rest of this
 * file exists to prevent, silently, on the way *out*. `JSON.rawJSON` is the
 * one mechanism that writes the digits verbatim.
 */
export function stringifyExact(value: unknown): string {
  return JSON.stringify(value, (_key: string, v: unknown) => {
    if (typeof v !== "bigint") return v;

    const raw = (JSON as { rawJSON?: (text: string) => unknown }).rawJSON;
    if (!raw) {
      throw new Error(
        "this runtime cannot emit exact 64-bit integers in JSON; a recent browser is required",
      );
    }
    return raw(v.toString());
  });
}
