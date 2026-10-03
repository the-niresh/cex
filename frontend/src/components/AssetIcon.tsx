import { assetColor } from "../lib/assets";

interface Props {
  asset: string;
  size?: number;
}

/**
 * Round coin mark per asset. Recognisable at a glance without loading images.
 */
export function AssetIcon({ asset, size = 24 }: Props) {
  const px = `${size}px`;
  const bg = assetColor(asset);

  return (
    <span
      className="flex flex-none items-center justify-center rounded-full"
      style={{ width: px, height: px, backgroundColor: bg }}
      aria-hidden="true"
    >
      {asset === "BTC" && (
        <svg width={size * 0.55} height={size * 0.55} viewBox="0 0 16 16" fill="none">
          <path
            d="M10.2 7.1c.7-.4 1.1-1.1 1-2-.1-1.3-1.1-1.7-2.3-1.8v-1.1h-.9v1.1h-.7v-1.1h-.9v1.1c-.2 0-.4 0-.6.1l-.1-1h-1l.2 1.3c-.1 0-.2.1-.2.1l-.2-.9h-1l.2 1c-.5.2-.8.5-.7 1 .1.7.6 1 1.2 1.1v.1c-.7.1-1.2.4-1.1 1.2.1.7.7 1 1.4 1.1v1.1h.9v-1.1h.7v1.1h.9v-1.1c.2 0 .4-.1.6-.1l.1 1h1l-.2-1.3c.1 0 .2-.1.3-.1l.2.9h1l-.2-1c.8-.3 1.3-.8 1.2-1.6-.1-.9-.7-1.3-1.5-1.5zm-2.3-2.4c.5 0 1.2.2 1.2.9 0 .8-.7.9-1.3 1v-1.9zm-1 3.8v-2c.6-.1 1.3-.2 1.3.9 0 1-.6 1.1-1.3 1.1z"
            fill="#fff"
          />
        </svg>
      )}
      {asset === "ETH" && (
        <svg width={size * 0.45} height={size * 0.55} viewBox="0 0 12 20" fill="none">
          <path d="M6 0 0 10.2 6 13.4V0z" fill="#fff" fillOpacity="0.6" />
          <path d="M6 0 12 10.2 6 13.4V0z" fill="#fff" />
          <path d="M6 14.6 0 11.4 6 20v-5.4z" fill="#fff" fillOpacity="0.6" />
          <path d="M6 14.6 12 11.4 6 20v-5.4z" fill="#fff" />
        </svg>
      )}
      {asset === "SOL" && (
        <svg width={size * 0.5} height={size * 0.5} viewBox="0 0 16 16" fill="none">
          <rect x="1" y="2" width="14" height="3" rx="1" fill="#fff" />
          <rect x="1" y="6.5" width="14" height="3" rx="1" fill="#fff" fillOpacity="0.7" />
          <rect x="1" y="11" width="14" height="3" rx="1" fill="#fff" fillOpacity="0.5" />
        </svg>
      )}
      {asset === "USDT" && (
        <svg width={size * 0.4} height={size * 0.5} viewBox="0 0 10 14" fill="none">
          <path
            d="M5.5 0v2.1c2.5.2 4.5 1.1 4.5 2.4 0 1.3-2 2.2-4.5 2.4v1.1h-1V7c-2.5-.2-4.5-1.1-4.5-2.4C0 3.3 2 2.4 4.5 2.1V0h1zm-.5 3.2c-1.8-.1-3.2-.7-3.2-1.5s1.4-1.4 3.2-1.5v3zm1 0V1.2c1.8.1 3.2.7 3.2 1.5s-1.4 1.4-3.2 1.5zm-1 4.3c-1.8-.1-3.2-.7-3.2-1.5h6.4c0 .8-1.4 1.4-3.2 1.5zm0 2.5v3.5h-1V10h1z"
            fill="#fff"
          />
        </svg>
      )}
      {!["BTC", "ETH", "SOL", "USDT"].includes(asset) && (
        <span className="font-sans text-micro font-medium text-bg">{asset.charAt(0)}</span>
      )}
    </span>
  );
}
