import { useEffect, useId, useRef, useState } from "react";
import {
  DEPOSIT_LIMITS,
  USDT_PRESETS,
  decimalsForAsset,
  depositAssetLabel,
  depositAssets,
} from "../lib/deposit";
import { formatAtoms, parseAtoms } from "../lib/num";
import type { Market } from "../lib/types";
import { ActionButton, AvailableLine, Segment, Segmented, FieldInput } from "./ui/form";
import { PanelHead, PanelTitle } from "./ui/panel";

interface Props {
  markets: Market[];
  signedIn: boolean;
  onGuest(): void;
  onDeposit(asset: string, amount: bigint): Promise<void>;
  onClose(): void;
}

export function DepositDialog({
  markets,
  signedIn,
  onGuest,
  onDeposit,
  onClose,
}: Props) {
  const assets = depositAssets(markets);
  const [asset, setAsset] = useState("USDT");
  const [amount, setAmount] = useState("10000");
  const [sending, setSending] = useState(false);
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);

  const decimals = decimalsForAsset(asset, markets);
  const parsed = parseAtoms(amount, decimals);
  const limit = DEPOSIT_LIMITS[asset];
  const ready = !sending && parsed !== null && parsed > 0n;

  useEffect(() => {
    panelRef.current?.focus();
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function credit() {
    if (!signedIn) {
      onGuest();
      return;
    }
    if (!ready || parsed === null) return;
    setSending(true);
    try {
      await onDeposit(asset, parsed);
      onClose();
    } finally {
      setSending(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-bg/80 backdrop-blur-[2px]"
      data-testid="deposit-dialog"
    >
      <div className="absolute inset-0" onClick={onClose} aria-hidden="true" />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="relative flex w-80 flex-col rounded-panel border border-rule bg-panel outline-none"
      >
        <PanelHead>
          <PanelTitle id={titleId}>Deposit {depositAssetLabel(asset)}</PanelTitle>
          <button
            type="button"
            className="ml-auto flex min-h-6 min-w-6 cursor-pointer items-center justify-center text-[11px] leading-none text-ink-4 hover:text-ink"
            aria-label="close"
            onClick={onClose}
          >
            x
          </button>
        </PanelHead>
        <div className="flex flex-col gap-4 p-3">
          <Segmented variant="control" className="grid-cols-4" data-testid="deposit-assets">
            {assets.map((a) => (
              <Segment key={a} quiet selected={a === asset} onClick={() => setAsset(a)}>
                {a}
              </Segment>
            ))}
          </Segmented>
          {asset === "USDT" && (
            <Segmented variant="control" className="grid-cols-3" data-testid="deposit-presets">
              {USDT_PRESETS.map((preset) => {
                const selected = amount === String(preset);
                return (
                  <Segment
                    key={preset}
                    quiet
                    tone={selected ? "buy" : "control"}
                    selected={selected}
                    onClick={() => setAmount(String(preset))}
                  >
                    {preset.toLocaleString()}
                  </Segment>
                );
              })}
            </Segmented>
          )}
          <div className="grid grid-cols-[minmax(0,1fr)_118px] gap-[7px]">
            <FieldInput
              bad={parsed === null && amount !== ""}
              value={amount}
              inputMode="decimal"
              aria-label="deposit amount"
              onChange={(e) => setAmount(e.target.value)}
            />
            <ActionButton
              disabled={signedIn && !ready}
              className="h-10 px-3 text-micro"
              onClick={() => void credit()}
            >
              {!signedIn ? "Try as guest" : sending ? "..." : "Confirm deposit"}
            </ActionButton>
          </div>
          {parsed !== null && (
            <AvailableLine label="credits">
              {formatAtoms(parsed, decimals)} {asset}
            </AvailableLine>
          )}
          {limit !== undefined && (
            <div className="font-sans text-micro text-ink-4">
              Limit: {limit.toLocaleString()} {asset} per deposit
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
