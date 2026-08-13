import { API_URL } from "../lib/api";
import { ENGINE_P99_BASELINE_MS, type LatencyStats, type Reading, type Sample } from "../lib/latency";
import { Sparkline } from "./Sparkline";

interface Props {
  stats: LatencyStats;
  series: Sample[];
  depthSeq: bigint | null;
  resyncs: number;
  status: string;
  /** Same flag the topbar reads — see the note on TopBar's own prop. */
  feedDegraded: boolean;
  silentForMs: number | null;
}

/**
 * What the exchange costs, measured rather than claimed, split by which
 * subsystem actually did the work.
 *
 * Three numbers, never summed. `engine` is time inside the matching engine.
 * `history` is a request the engine never saw — `/trades` and `/candles` come
 * from Postgres — and is dominated by that database's round trip, not by
 * anything the exchange does. `network` is what the wire added, and describes
 * where the reader is sitting rather than the exchange at all.
 *
 * Reported as p50 over a rolling fifty requests, so one lucky call cannot
 * flatter the readout, with p99 beside each so one unlucky one is not hidden.
 */
export function InstrumentStrip({
  stats,
  series,
  depthSeq,
  resyncs,
  status,
  feedDegraded,
  silentForMs,
}: Props) {
  // Amber above the p99 the exchange actually measured under load, not above a
  // number someone picked. Only the engine gets this: no baseline was ever
  // measured for the database path, and inventing one is the exact habit this
  // readout exists to argue against.
  //
  // Distinct from `feedDegraded`: this is the exchange being slow, that is the
  // book being untrustworthy. Both go amber, for different reasons.
  const engineSlow = stats.engine.p50 !== null && stats.engine.p50 > ENGINE_P99_BASELINE_MS;

  return (
    <footer
      data-testid="statusbar"
      className={[
        // The strip is the floor of the screen, not a floating card — but it is
        // still a surface, so it takes the same border and radius as one.
        "col-span-full flex items-stretch gap-5 rounded-panel border border-rule bg-panel-hi px-3",
        // Stacked on a phone it has to wrap rather than push the page sideways.
        "max-[879px]:h-auto max-[879px]:min-h-[22px] max-[879px]:flex-wrap max-[879px]:gap-y-0.5",
      ].join(" ")}
    >
      <Gauge
        label="engine"
        testid="status-engine"
        unit="ms"
        reading={stats.engine}
        values={series.map((s) => s.engineMs)}
        stroke={engineSlow ? "var(--color-warn)" : "var(--color-buy)"}
        tone={engineSlow ? "text-warn" : "text-ink"}
        title="Time inside the matching engine"
      />

      <Gauge
        label="history"
        testid="status-history"
        unit="ms"
        reading={stats.history}
        values={series.map((s) => s.historyMs)}
        stroke="var(--color-ink-3)"
        tone="text-ink-2"
        title="Requests served from Postgres, which the engine never sees"
      />

      <Gauge
        label="network"
        testid="status-network"
        unit="ms"
        reading={stats.network}
        values={series.map((s) => s.networkMs)}
        stroke="var(--color-control)"
        tone="text-ink-2"
        title="What the wire added, between this browser and the API"
      />

      <div className="ml-auto flex items-center gap-4 self-center text-micro text-ink-4">
        {/* Which exchange this screen is actually talking to. Worth keeping
            visible: localhost and the deployed host look identical otherwise. */}
        <Fact label="api" value={new URL(API_URL).host} />
        <Fact label="depth_seq" value={depthSeq === null ? "—" : String(depthSeq)} testid="status-depth-seq" />
        <Fact label="resyncs" value={String(resyncs)} testid="status-resyncs" />
        <Fact
          label="updated"
          value={silentForMs === null ? "—" : `${Math.floor(silentForMs / 1000)}s`}
          testid="status-updated"
        />
        <span className="flex items-center gap-1.5">
          <i
            className={`size-1.5 rounded-full ${feedDegraded ? "bg-warn" : "bg-buy"}`}
            aria-hidden
          />
          <span className="uppercase tracking-[0.12em] text-ink-3">{status}</span>
        </span>
      </div>
    </footer>
  );
}

function Gauge({
  label,
  testid,
  unit,
  reading,
  values,
  stroke,
  tone,
  title,
}: {
  label: string;
  testid: string;
  unit: string;
  reading: Reading;
  /** Nulls are the samples belonging to the other readings; they are skipped. */
  values: (number | null)[];
  stroke: string;
  tone: string;
  title: string;
}) {
  const own = values.filter((v): v is number => v !== null);

  return (
    <div className="flex items-center gap-2 py-2" title={title}>
      <div className="w-[4.25rem]">
        <div className="text-micro uppercase tracking-[0.14em] text-ink-4">{label}</div>
        {/* Em-dash, never a zero: a missing measurement must not render as a
            fast one. That is the whole reason this readout exists. */}
        <div className={`tnum text-[0.9375rem] leading-tight ${tone}`} data-testid={testid}>
          {reading.p50 === null ? "—" : `${reading.p50}${unit}`}
        </div>
      </div>
      <Sparkline values={own} stroke={stroke} label={label} width={104} />
      <div className="w-[4.75rem] shrink-0 text-micro text-ink-4">
        <div className="uppercase tracking-[0.1em]">p99 · n</div>
        <div className="tnum whitespace-nowrap text-ink-3">
          {reading.p99 === null ? "—" : `${reading.p99}`}
          <span className="text-ink-4"> · {reading.count}</span>
        </div>
      </div>
    </div>
  );
}

function Fact({ label, value, testid }: { label: string; value: string; testid?: string }) {
  return (
    <span className="flex items-baseline gap-1.5">
      <span className="uppercase tracking-[0.1em]">{label}</span>
      <b className="tnum font-normal text-ink-2" data-testid={testid}>
        {value}
      </b>
    </span>
  );
}
