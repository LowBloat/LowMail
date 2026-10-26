<script lang="ts">
	/**
	 * Shows a sanitized message body in a sandboxed frame: no scripts, its own
	 * Content-Security-Policy (remote images only when allowed), links opened
	 * outside the app, height following the content.
	 */
	import { openExternal } from "../../api";
	import { frameTheme } from "../../frame-theme";
	import { compose } from "../../state/compose.svelte";

	let { html, remote = false, title, dark = false }: { html: string; remote?: boolean; title: string; dark?: boolean } = $props();

	let frame: HTMLIFrameElement | undefined = $state();
	let height = $state(120);
	let observer: ResizeObserver | null = null;

	const plainOnly = $derived(/^<div class="lm-plain"/.test(html.trim()));
	const darkText = $derived(dark && plainOnly);

	const doc = $derived.by(() => {
		const remoteSrc = remote ? " https: http:" : "";
		const csp = `default-src 'none'; img-src data: cid:${remoteSrc}; style-src 'unsafe-inline'${remoteSrc}; font-src data:${remoteSrc}; media-src data:`;
		// HTML mail is designed for white paper; plain text follows the app.
		const theme = frameTheme(darkText ? "dark" : "light");
		const colors = `color:${theme.text};background:${darkText ? theme.surface : "white"}`;
		const [padY, padX] = theme.pad.roomy;
		return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><meta name="referrer" content="no-referrer"><meta http-equiv="x-dns-prefetch-control" content="off"><style>
html{${colors}}
body{margin:0;padding:${padY} ${padX};font:${theme.font}/1.55 ${theme.family};overflow-wrap:break-word;${colors}}
img{max-width:100%;height:auto}
pre{white-space:pre-wrap}
a{color:${theme.link}}
blockquote[type=cite]{margin:0 0 0 .8ex;padding-left:1ex;border-left:${theme.stroke} solid ${theme.quote}}
.lm-plain{white-space:pre-wrap}
.lm-plain blockquote[type=cite]{color:${theme.muted}}
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
		// Middle clicks would open links inside the app.
		d.addEventListener("auxclick", (e) => {
			if ((e.target as Element | null)?.closest?.("a[href]")) e.preventDefault();
		});
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
	sandbox="allow-same-origin"
	referrerpolicy="no-referrer"
	style:height="{height}px"
	onload={onLoad}
></iframe>

<style>
	.lm-frame {
		display: block;
		width: 100%;
		border: 0;
		background: white;
	}
</style>
