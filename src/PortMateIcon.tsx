import { useId } from "react";

export interface PortMateIconProps {
  size?: number;
  className?: string;
  variant?: "mark" | "badge";
}

export default function PortMateIcon({
  size = 20,
  className,
  variant = "badge",
}: PortMateIconProps) {
  const uid = useId().replace(/:/g, "");
  const bgId = `pm-bg-${uid}`;
  const glowId = `pm-glow-${uid}`;
  const rimId = `pm-rim-${uid}`;
  const glyphId = `pm-glyph-${uid}`;
  const chamberId = `pm-chamber-${uid}`;

  if (variant === "mark") {
    return (
      <svg
        width={size}
        height={size}
        viewBox="0 0 64 64"
        fill="none"
        className={className}
        aria-hidden="true"
      >
        <defs>
          <linearGradient id={bgId} x1="6" y1="4" x2="58" y2="60" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#0f1d36" />
            <stop offset="100%" stopColor="#081222" />
          </linearGradient>
          <linearGradient id={rimId} x1="8" y1="6" x2="56" y2="58" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#38bdf8" stopOpacity="0.92" />
            <stop offset="100%" stopColor="#2dd4bf" stopOpacity="0.88" />
          </linearGradient>
        </defs>
        <rect x="4" y="4" width="56" height="56" rx="14" fill={`url(#${bgId})`} />
        <rect x="5.2" y="5.2" width="53.6" height="53.6" rx="12.8" stroke={`url(#${rimId})`} strokeWidth="2.4" />
        {/* Two crisp hardware pins + keyed bottom notch */}
        <path d="M23 10.5V15.5M34 10.5V15.5" stroke="#38bdf8" strokeWidth="3.2" strokeLinecap="round" />
        <rect x="26" y="49.5" width="12" height="3.8" rx="1.4" fill="#2dd4bf" />
        {/* Keyed connector chamber (upper-right clip) */}
        <path
          d="M17.5 16H39.5L48.5 25V44.5C48.5 46.2 47.2 47.5 45.5 47.5H17.5C15.8 47.5 14.5 46.2 14.5 44.5V19C14.5 17.3 15.8 16 17.5 16Z"
          stroke="#38bdf8"
          strokeOpacity="0.58"
          strokeWidth="2.2"
          strokeLinejoin="round"
        />
        {/* High-contrast prompt > + circuit trace to square contact pad */}
        <path d="M21 24.5L30 32L21 39.5" stroke="#67e8f9" strokeWidth="4.4" strokeLinecap="round" strokeLinejoin="round" />
        <path d="M31.5 38.5H38.5" stroke="#38bdf8" strokeWidth="3.6" strokeLinecap="round" />
        <rect x="38" y="34.5" width="8" height="8" rx="1.6" fill="#2dd4bf" />
      </svg>
    );
  }

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
      className={className}
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={bgId} x1="6" y1="4" x2="58" y2="60" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#0f1d36" />
          <stop offset="52%" stopColor="#0b162a" />
          <stop offset="100%" stopColor="#071020" />
        </linearGradient>
        <radialGradient id={glowId} cx="46%" cy="44%" r="54%">
          <stop offset="0%" stopColor="#38bdf8" stopOpacity="0.34" />
          <stop offset="52%" stopColor="#2563eb" stopOpacity="0.14" />
          <stop offset="100%" stopColor="#0b1526" stopOpacity="0" />
        </radialGradient>
        <linearGradient id={rimId} x1="8" y1="6" x2="56" y2="58" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#38bdf8" stopOpacity="0.82" />
          <stop offset="50%" stopColor="#60a5fa" stopOpacity="0.34" />
          <stop offset="100%" stopColor="#2dd4bf" stopOpacity="0.78" />
        </linearGradient>
        <linearGradient id={chamberId} x1="15" y1="15" x2="49" y2="49" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#38bdf8" stopOpacity="0.72" />
          <stop offset="100%" stopColor="#2dd4bf" stopOpacity="0.48" />
        </linearGradient>
        <linearGradient id={glyphId} x1="20" y1="24" x2="46" y2="41" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#a5f3fc" />
          <stop offset="48%" stopColor="#38bdf8" />
          <stop offset="100%" stopColor="#2dd4bf" />
        </linearGradient>
      </defs>
      <rect x="3" y="3" width="58" height="58" rx="15" fill={`url(#${bgId})`} />
      <rect x="3" y="3" width="58" height="58" rx="15" fill={`url(#${glowId})`} />
      <rect x="3.75" y="3.75" width="56.5" height="56.5" rx="14.25" stroke={`url(#${rimId})`} strokeWidth="1.5" />
      <path d="M21 9.5V14.5M28 8V14.5M35 9.5V14.5" stroke="#38bdf8" strokeOpacity="0.86" strokeWidth="2.1" strokeLinecap="round" />
      <path d="M21 49.5V54.5M43 49.5V54.5" stroke="#2dd4bf" strokeOpacity="0.78" strokeWidth="2.1" strokeLinecap="round" />
      <rect x="27" y="50" width="10" height="3.2" rx="1.2" fill="#2dd4bf" fillOpacity="0.78" />
      <path d="M9.5 26H13.5M9.5 38H13.5M50.5 30H54.5M50.5 39H54.5" stroke="#60a5fa" strokeOpacity="0.52" strokeWidth="1.75" strokeLinecap="round" />
      <path
        d="M19 15H39.5L49 24.5V44C49 46.2 47.2 48 45 48H37.5V45.4H26.5V48H19C16.8 48 15 46.2 15 44V19C15 16.8 16.8 15 19 15Z"
        fill="#081222"
        fillOpacity="0.88"
        stroke={`url(#${chamberId})`}
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
      <path d="M17.5 21V19C17.5 18.2 18.2 17.5 19 17.5H38" stroke="#7dd3fc" strokeOpacity="0.28" strokeWidth="1.1" strokeLinecap="round" />
      <path d="M21.5 25L29.5 32L21.5 39" stroke={`url(#${glyphId})`} strokeWidth="3.4" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M31.5 38H39.5" stroke={`url(#${glyphId})`} strokeWidth="2.9" strokeLinecap="round" />
      <rect x="39" y="34.75" width="6.5" height="6.5" rx="1.4" fill="#2dd4bf" />
      <circle cx="40.5" cy="24.5" r="2.1" fill="#67e8f9" />
      <circle cx="40.5" cy="24.5" r="4.2" fill="#38bdf8" fillOpacity="0.22" />
    </svg>
  );
}
