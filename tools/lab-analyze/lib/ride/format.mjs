// Text output of `lab-analyze ride`.

const pad = (s, w) => String(s ?? "-").padEnd(w);
const types = (m) =>
  Object.entries(m ?? {})
    .sort((a, b) => b[1] - a[1])
    .map(([k, v]) => `${k}:${v}`)
    .join(" ");

/** The full report of one ride as text. */
export function formatRide(r) {
  const out = [];
  out.push(`=== ride ${r.rideKey} — ${r.alignment.length} device file(s)`);
  out.push("-- files and alignment");
  for (const a of r.alignment) {
    out.push(`  ${pad(a.device, 14)} ${pad(a.model, 16)} ${pad(a.sessionId, 22)} ${a.method} corr=${a.corrMs} ms${a.residualMs === null ? "" : ` residual=${a.residualMs} ms`}`);
  }
  out.push(`-- timeline (${r.timelineAll ? "all events" : "warn-level events and marks; --all for everything"})`);
  for (const t of r.timeline) out.push(`  ${t.time}  ${pad(t.device, 14)} ${pad(t.what, 16)} ${t.text}`);
  out.push("-- per-link traffic per minute (kbit/s; far = the other end of the same link)");
  for (const l of r.traffic.links) {
    out.push(
      `  ${l.minute} ${pad(l.device, 14)} link=${l.link} peer=${l.peer ?? "?"} ${l.kind} tx=${l.txKbps} rx=${l.rxKbps} far tx/rx=${l.farTxKbps ?? "-"}/${l.farRxKbps ?? "-"} loss=${l.loss ?? "-"} rttP95=${l.rttP95 ?? "-"} wb=${l.wb} qMax=${l.qMax} grant=${l.grantKbps ?? "-"} | tx ${types(l.txTypes)}`,
    );
  }
  out.push("-- idle overhead per node (non-voice kbit/s sent)");
  for (const o of r.traffic.idle) out.push(`  ${o.minute} ${pad(o.device, 14)} nonVoice=${o.nonVoiceKbps} voice=${o.voiceKbps}`);
  out.push("-- audio");
  for (const a of r.audio) {
    const under = a.underPerMin.map((m) => `${m.minute}:${m.under}/${m.over}`).join(" ");
    out.push(`  ${pad(a.device, 14)} under/over per min ${under || "-"}; cbSizeChanges=${a.cbSizeChanges} late=${a.latePer1000 ?? "-"}‰ plc=${a.plcPer1000 ?? "-"}‰ fec=${a.fecPer1000 ?? "-"}‰ jbTarget=${a.jbTargetMs ? a.jbTargetMs.join("-") : "-"} ms routeChanges=${a.routeChanges} glitches=${a.glitches}`);
    if (a.perms.length) out.push(`    perms: ${a.perms.join(", ")}`);
    for (const t of a.transitions) out.push(`    ${t}`);
    if (a.mismatch) out.push(`    MISMATCH: ${a.mismatch}`);
  }
  out.push("-- partitions");
  for (const p of r.partitions.islands) out.push(`  ${p.time} ${pad(p.device, 14)} islands=${p.islands} mine=${p.mine}`);
  for (const m of r.partitions.matrix) out.push(`  ${m.minute} ${pad(m.device, 14)} hears [${m.heard.join(",")}] islandsMax=${m.islandsMax}`);
  out.push("-- marks (±10 s)");
  for (const m of r.marks) {
    out.push(`  ${m.mark.time} ${m.mark.device} MARK ${m.mark.text}`);
    for (const c of m.context) out.push(`      ${c.time} ${pad(c.device, 14)} ${c.what} ${c.text}`);
  }
  return out.join("\n");
}
