/** Prints one message with its headers, from a hidden frame. */
import { escapeHtml, formatAddress, fullDate } from "./format";
import { t } from "./i18n/index.svelte";
import type { Message } from "./types";

export function printMessage(m: Message) {
	const rows: [string, string][] = [
		[t("message.from"), m.from.map(formatAddress).join(", ")],
		[t("message.to"), m.to.map(formatAddress).join(", ")],
	];
	if (m.cc.length) rows.push([t("message.cc"), m.cc.map(formatAddress).join(", ")]);
	rows.push([t("message.date"), fullDate(m.date)]);
	const head = rows.map(([k, v]) => `<tr><th>${escapeHtml(k)}</th><td>${escapeHtml(v)}</td></tr>`).join("");
	const files = m.attachments.filter((a) => !a.inline).map((a) => escapeHtml(a.filename));
	const doc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'"><title>${escapeHtml(m.subject)}</title><style>
body{font:13px/1.5 "Inter Variable",Inter,"Segoe UI",system-ui,sans-serif;color:#000;margin:24px}
h1{font-size:18px;margin:0 0 10px}
table.h{border-collapse:collapse;margin-bottom:14px}
table.h th{text-align:left;padding:1px 12px 1px 0;color:#444;font-weight:600;vertical-align:top}
hr{border:0;border-top:1px solid #999;margin:12px 0}
img{max-width:100%}
.lm-plain{white-space:pre-wrap}
blockquote[type=cite]{margin:0 0 0 .8ex;padding-left:1ex;border-left:2px solid #999}
</style></head><body><h1>${escapeHtml(m.subject || t("message.noSubject"))}</h1><table class="h">${head}</table><hr>${m.html ?? ""}${files.length ? `<hr><p><b>${escapeHtml(t("message.attachments"))}:</b> ${files.join(", ")}</p>` : ""}</body></html>`;
	const frame = document.createElement("iframe");
	frame.setAttribute("sandbox", "allow-same-origin allow-modals");
	frame.style.cssText = "position:fixed;width:0;height:0;border:0;right:0;bottom:0";
	frame.srcdoc = doc;
	frame.onload = () => {
		frame.contentWindow?.focus();
		frame.contentWindow?.print();
		setTimeout(() => frame.remove(), 60_000);
	};
	document.body.appendChild(frame);
}
