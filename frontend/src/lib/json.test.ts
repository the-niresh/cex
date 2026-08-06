import { describe, expect, it } from "vitest";
import { parseExact, stringifyExact } from "./json";

describe("exact integers", () => {
  it("reads an integer past the safe range without rounding", () => {
    const parsed = parseExact('{"balance": 9007199254740993}') as { balance: bigint };

    expect(parsed.balance).toBe(9007199254740993n);
    // What the naive path would have done, kept here so the point is not lost.
    expect(JSON.parse('{"balance": 9007199254740993}').balance).toBe(9007199254740992);
  });

  it("writes an integer past the safe range without rounding", () => {
    expect(stringifyExact({ amount: 9007199254740993n })).toBe('{"amount":9007199254740993}');
  });

  it("survives a round trip", () => {
    const original = { price: 50000000000n, qty: 100000n, deposit: 9007199254740993n };
    expect(parseExact(stringifyExact(original))).toEqual(original);
  });

  it("leaves strings and booleans alone", () => {
    expect(parseExact('{"a":"12","b":true,"c":null}')).toEqual({ a: "12", b: true, c: null });
  });

  it("keeps a negative integer exact", () => {
    expect((parseExact('{"d": -9007199254740993}') as { d: bigint }).d).toBe(-9007199254740993n);
  });
});
