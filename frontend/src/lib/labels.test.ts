import { describe, expect, it } from "vitest";
import { sentenceCase } from "./labels";

describe("sentenceCase", () => {
  it("turns a one-word enum into a word", () => {
    expect(sentenceCase("OPEN")).toBe("Open");
    expect(sentenceCase("LIMIT")).toBe("Limit");
    expect(sentenceCase("TAKER")).toBe("Taker");
  });

  it("turns underscores into spaces and capitalises only the first word", () => {
    expect(sentenceCase("PARTIALLY_FILLED")).toBe("Partially filled");
  });

  it("covers every status the engine can send", () => {
    // crates/proto/src/lib.rs — OrderStatus, SCREAMING_SNAKE_CASE on the wire.
    expect(["OPEN", "PARTIALLY_FILLED", "FILLED", "CANCELLED", "REJECTED"].map(sentenceCase)).toEqual(
      ["Open", "Partially filled", "Filled", "Cancelled", "Rejected"],
    );
  });

  it("reads a status nobody has added yet, rather than falling back to shouting", () => {
    expect(sentenceCase("EXPIRED_AT_AUCTION")).toBe("Expired at auction");
  });

  it("leaves a value it cannot improve alone", () => {
    expect(sentenceCase("")).toBe("");
    expect(sentenceCase("_")).toBe("_");
  });
});
