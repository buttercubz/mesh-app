// Shared row geometry of the rider lists (Members screen, lobby ready list). The rider's
// name is the key information: it must stay readable at large text sizes instead of being
// cut to a few letters or crushed into a column a third of the screen wide. The parent
// sets `@container`; three layouts follow the row width (in rem, so they also follow the
// text size):
// - wide (≥ 18 rem): avatar on the left, name and details in a column beside it;
// - narrow (≥ 12.5 rem): avatar, name and trailing chevron share the first line, the
//   badges and details take the full row width below;
// - very narrow: avatar and chevron on the first line, the name on its own full line,
//   then the details.
import { cx } from "@meshapp/ui-kit";
import type { ReactNode } from "react";

export interface RiderRowLayoutProps {
  readonly avatar: ReactNode;
  /** The name line (key information: wraps, never truncates to a few letters). */
  readonly name: ReactNode;
  readonly details: ReactNode;
  /** Optional trailing affordance (chevron of an actionable row). */
  readonly trailing?: ReactNode;
}

export function RiderRowLayout({ avatar, name, details, trailing }: RiderRowLayoutProps) {
  return (
    <span
      className={cx(
        "grid w-full items-center gap-x-3 gap-y-1.5 @min-[18rem]:items-start",
        trailing === undefined ? "grid-cols-[auto_minmax(0,1fr)]" : "grid-cols-[auto_minmax(0,1fr)_auto]",
      )}
    >
      <span className="col-start-1 row-start-1 flex @min-[18rem]:row-end-3">{avatar}</span>
      <span className="col-[1/-1] row-start-2 flex min-w-0 @min-[12.5rem]:col-[2/3] @min-[12.5rem]:row-start-1">{name}</span>
      {trailing !== undefined && (
        <span className="col-[-2/-1] row-start-1 flex justify-self-end @min-[18rem]:row-end-3 @min-[18rem]:mt-3">{trailing}</span>
      )}
      <span className="col-[1/-1] row-start-3 flex min-w-0 flex-col gap-1.5 @min-[12.5rem]:row-start-2 @min-[18rem]:col-[2/3]">
        {details}
      </span>
    </span>
  );
}
