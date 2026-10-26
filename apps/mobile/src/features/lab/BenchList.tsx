// Bench list with the go/no-go thresholds (WP-21, DESIGN §12.2, §12.5). Phase 0 benches
// first, then the Phase 1 additions (badged). A row opens the bench's config sheet.
import { Badge, ListRow, ListSection } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";
import type { BenchSpec } from "./catalog";
import { BENCH_CATALOG } from "./catalog";
import { criterionParams } from "./thresholds";

export function BenchList({ onOpen }: { readonly onOpen: (spec: BenchSpec) => void }) {
  const { t } = useT("lab");
  return (
    <ListSection title={t("section.benches")}>
      <div data-testid="lab-benches">
        {BENCH_CATALOG.map((spec) => (
          <ListRow
            key={spec.bench}
            ariaLabel={t("bench.open", { bench: spec.bench })}
            title={
              // The phase badge flows after the title, so the title keeps the full row
              // width on narrow screens and large text instead of beside a trailing badge.
              <span>
                <span className="font-semibold">{spec.bench}</span> · {t(`bench.${spec.bench}.title`)}
                {spec.phase > 0 && (
                  <Badge tone="brand" className="ms-1.5 align-middle">
                    {t("bench.phase", { phase: spec.phase })}
                  </Badge>
                )}
              </span>
            }
            subtitle={t(`bench.${spec.bench}.criterion`, criterionParams(spec.bench))}
            trailing={<span aria-hidden="true">›</span>}
            onClick={() => onOpen(spec)}
          />
        ))}
      </div>
    </ListSection>
  );
}
