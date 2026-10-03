import { useState } from "react";
import {
  DEPOSIT_LIMITS,
  USDT_PRESETS,
  decimalsForAsset,
  depositAssetLabel,
  depositAssets,
} from "../lib/deposit";
import { formatAtoms, parseAtoms } from "../lib/num";
import type { Market } from "../lib/types";
import { AvailableLine, GhostButton, Segment, Segmented, FieldInput } from "./ui/form";
import { PanelHead, PanelTitle } from "./ui/panel";

interface Props {
  markets: Market[];
  signedIn: boolean;
  onRequireSignIn(): void;
  onDeposit(asset: string, amount: bigint): Promise<void>;
  onClose(): void;
}

export function DepositDialog({
  markets,
  signedIn,
  onRequireSignIn,
  onDeposit,
  onClose,
}: Props) {
  const assets = depositAssets(markets);
  const [asset, setAsset] = useState("USDT");
  const [amount, setAmount] = useState("10000");
  const [sending, setSending] = useState(false);

  const decimals = decimalsForAsset(asset, markets);
  const parsed = parseAtoms(amount, decimals);
  const limit = DEPOSIT_LIMITS[asset];
  const ready = !sending && parsed !== null && parsed > 0n;

  async function credit() {
    if (!signedIn) {
      onRequireSignIn();
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
      <div className="relative flex w-80 flex-col rounded-panel border border-rule bg-panel">
        <PanelHead>
          <PanelTitle>Deposit {depositAssetLabel(asset)}</PanelTitle>
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
              {USDT_PRESETS.map((preset) => (
                <Segment
                  key={preset}
                  quiet
                  selected={amount === String(preset)}
                  onClick={() => setAmount(String(preset))}
                >
                  {preset.toLocaleString()}
                </Segment>
              ))}
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
            <GhostButton disabled={signedIn && !ready} onClick={() => void credit()}>
              {!signedIn ? "Log in" : sending ? "..." : "Confirm"}
            </GhostButton>
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
