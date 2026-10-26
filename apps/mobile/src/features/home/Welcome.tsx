// First-launch welcome on Home (WP-22, DESIGN §1.3 F0, §0.2): what the app is, the
// honest range promise; the way into onboarding (about a minute, foreground) is the
// "Get started" button HomeScreen pins in its footer.
import type { LucideIcon } from "lucide-react";
import { BellRing, QrCode, Smartphone } from "lucide-react";

import { useT } from "../../i18n/index";

/** The app's mark: three riders linked by voice, in the brand colour. */
export function BrandLogo({ className = "size-8" }: { readonly className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={className} aria-hidden="true" fill="none">
      <rect width="32" height="32" rx="9" className="fill-brand" />
      <path d="M9 21 L16 10 L23 21" className="stroke-brand-foreground" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
      <circle cx="9" cy="21" r="3.2" className="fill-brand-foreground" />
      <circle cx="16" cy="10" r="3.2" className="fill-brand-foreground" />
      <circle cx="23" cy="21" r="3.2" className="fill-brand-foreground" />
    </svg>
  );
}

/** Logo plus the app name, for the Home title bar. */
export function BrandMark({ label }: { readonly label: string }) {
  return (
    <span className="inline-flex items-center gap-2.5 text-xl font-bold tracking-tight">
      <BrandLogo className="size-8 shrink-0" />
      <span className="truncate">{label}</span>
    </span>
  );
}

/** Hero illustration: riders on a road, linked by voice waves. */
function WelcomeArt() {
  return (
    <svg viewBox="0 0 240 140" className="mx-auto h-36 w-full max-w-72 text-brand" aria-hidden="true" fill="none">
      <defs>
        <radialGradient id="welcome-glow" cx="50%" cy="50%" r="50%">
          <stop offset="0" stopColor="currentColor" stopOpacity="0.35" />
          <stop offset="1" stopColor="currentColor" stopOpacity="0" />
        </radialGradient>
      </defs>
      <ellipse cx="120" cy="70" rx="118" ry="68" fill="url(#welcome-glow)" />
      <path d="M10 118 C 70 118, 90 60, 130 60 S 200 96, 232 52" className="stroke-foreground" strokeOpacity="0.12" strokeWidth="18" strokeLinecap="round" />
      <path d="M10 118 C 70 118, 90 60, 130 60 S 200 96, 232 52" className="stroke-foreground" strokeOpacity="0.35" strokeWidth="2" strokeDasharray="6 9" strokeLinecap="round" />
      {[
        [48, 108],
        [130, 60],
        [204, 74],
      ].map(([x, y], i) => (
        <g key={i}>
          <circle cx={x} cy={y} r="22" className="stroke-brand" strokeOpacity="0.25" strokeWidth="2" />
          <circle cx={x} cy={y} r="13" className="fill-brand" />
          <circle cx={x} cy={y} r="5" className="fill-brand-foreground" />
        </g>
      ))}
      <path d="M72 96 Q 96 74 110 70" className="stroke-brand" strokeWidth="2.5" strokeLinecap="round" strokeDasharray="2 6" />
      <path d="M152 64 Q 172 64 184 70" className="stroke-brand" strokeWidth="2.5" strokeLinecap="round" strokeDasharray="2 6" />
    </svg>
  );
}

export function Welcome() {
  const { t } = useT("onboarding");
  const { t: tc } = useT("common");
  const points: ReadonlyArray<readonly [LucideIcon, string]> = [
    [Smartphone, t("welcome.point1")],
    [QrCode, t("welcome.point2")],
    [BellRing, t("welcome.point3")],
  ];
  return (
    <section className="mx-auto flex max-w-md flex-col gap-7 pt-4 text-center" data-testid="welcome">
      <WelcomeArt />
      <div className="flex flex-col gap-2">
        <h2 className="text-3xl font-bold tracking-tight text-balance">{t("welcome.title")}</h2>
        <p className="text-lg text-muted-foreground">{tc("shell.tagline")}</p>
      </div>
      <ul className="flex flex-col gap-4 text-start text-base">
        {points.map(([Icon, text]) => (
          <li key={text} className="flex items-start gap-3">
            <span aria-hidden="true" className="grid size-10 shrink-0 place-items-center rounded-full bg-brand-soft text-brand">
              <Icon className="size-5" />
            </span>
            <span className="min-w-0 pt-2 text-pretty">{text}</span>
          </li>
        ))}
      </ul>
      <p className="text-xs leading-relaxed text-muted-foreground">{t("promise.short")}</p>
    </section>
  );
}
