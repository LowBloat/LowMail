<script lang="ts">
	/**
	 * Shows a sanitized message body in a sandboxed frame: no scripts, its own
	 * Content-Security-Policy (remote images only when allowed), links opened
	 * outside the app, height following the content.
	 */
	import { openExternal } from "../../api";
	import { compose } from "../../state/compose.svelte";

	let { html, remote = false, title, dark = false }: { html: string; remote?: boolean; title: string; dark?: boolean } = $props();

	let frame: HTMLIFrameElement | undefined = $state();
	let height = $state(120);
	let observer: ResizeObserver | null = null;

	const plainOnly = $derived(/^<div class="lm-plain">/.test(html.trim()));
	const darkText = $derived(dark && plainOnly);

	const doc = $derived.by(() => {
		const remoteSrc = remote ? " https: http:" : "";
		const csp = `default-src 'none'; img-src data: cid:${remoteSrc}; style-src 'unsafe-inline'${remoteSrc}; font-src data:${remoteSrc}; media-src data:`;
		const colors = darkText ? "color:#fbfcf8;background:#1d221c" : "color:#171a17;background:#ffffff";
		const quote = darkText ? "#41473e" : "#a9b1a5";
		return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><meta name="referrer" content="no-referrer"><base target="_blank"><style>
html{${colors}}
body{margin:0;padding:18px 22px;font:14px/1.55 "Inter Variable",Inter,"Segoe UI",system-ui,sans-serif;overflow-wrap:break-word;${colors}}
img{max-width:100%;height:auto}
pre{white-space:pre-wrap}
a{color:${darkText ? "#d4fc8a" : "#3d5c00"}}
blockquote[type=cite]{margin:0 0 0 .8ex;padding-left:1ex;border-left:2px solid ${quote}}
.lm-plain{white-space:pre-wrap;font-family:"Cascadia Mono",Consolas,"SF Mono",monospace;font-size:13px}
.lm-plain blockquote[type=cite]{color:${darkText ? "#b8c0b0" : "#4f584f"}}
</style></head><body>${html}</body></html>`;
	});

	function fit() {
		const d = frame?.contentDocument;
		if (!d?.documentElement) return;
		height = Math.max(60, Math.ceil(d.documentElement.scrollHeight));
	}

	function onLoad() {
		const d = frame?.contentDocument;
		if (!d) return;
		fit();
		observer?.disconnect();
		observer = new ResizeObserver(() => fit());
		observer.observe(d.documentElement);
		for (const img of Array.from(d.images)) img.addEventListener("load", fit, { once: true });
		// Show where links go (phishing links hide behind other text).
		for (const a of Array.from(d.querySelectorAll<HTMLAnchorElement>("a[href]"))) {
			if (!a.title) a.title = a.getAttribute("href") ?? "";
		}
		d.addEventListener("click", (e) => {
			const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
			if (!a) return;
			const href = a.getAttribute("href") ?? "";
			e.preventDefault();
			if (href.startsWith("#")) {
				d.getElementById(href.slice(1))?.scrollIntoView();
			} else if (/^mailto:/i.test(href)) {
				void compose.mailto(href);
			} else if (/^(https?|tel|sms):/i.test(href)) {
				void openExternal(href);
			}
		});
	}

	$effect(() => () => observer?.disconnect());
</script>

<iframe
	bind:this={frame}
	class="lm-frame"
	{title}
	srcdoc={doc}
	sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
	referrerpolicy="no-referrer"
	style:height="{height}px"
	onload={onLoad}
></iframe>

<style>
	.lm-frame {
		display: block;
		width: 100%;
		border: 0;
		background: #ffffff;
	}
</style>
