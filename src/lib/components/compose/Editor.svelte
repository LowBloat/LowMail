<script lang="ts">
	/**
	 * Rich-text editor: an editable document inside a sandboxed frame, so
	 * quoted mail keeps its look without restyling the app, and nothing pasted
	 * can run. The toolbar drives it with editing commands.
	 */
	import { Popover, Tooltip } from "@codeatlasdev/ui";
	import {
		AlignCenter,
		AlignLeft,
		AlignRight,
		Bold,
		Image,
		IndentDecrease,
		IndentIncrease,
		Italic,
		Link,
		List,
		ListOrdered,
		Palette,
		Quote,
		RemoveFormatting,
		Strikethrough,
		Type,
		Underline,
	} from "@lucide/svelte";
	import { frameTheme } from "../../frame-theme";
	import { prompt } from "../../state/dialogs.svelte";
	import { t } from "../../i18n/index.svelte";

	let {
		html,
		version = 0,
		onchange,
		minHeight = 240,
		placeholder = "",
		onkeydown,
	}: {
		html: string;
		/** Bumped when `html` was replaced from outside and must be reloaded. */
		version?: number;
		onchange: (html: string) => void;
		minHeight?: number;
		placeholder?: string;
		onkeydown?: (e: KeyboardEvent) => void;
	} = $props();

	let frame: HTMLIFrameElement | undefined = $state();
	let doc: Document | null = null;
	let loadedVersion = -1;
	let timer: ReturnType<typeof setTimeout> | undefined;
	let colorsOpen = $state(false);
	let sizesOpen = $state(false);
	let fileInput: HTMLInputElement | undefined = $state();
	let states = $state<Record<string, boolean>>({});

	const COLORS = ["#171a17", "#4f584f", "#b42318", "#c2410c", "#a16207", "#247a48", "#4d7100", "#1f5fa8", "#6d3fa0", "#be185d"];

	// The message is written on white paper, as most readers will see it.
	const paper = frameTheme("light");
	const [padY, padX] = paper.pad.compact;
	const shell = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: blob: https: http:; style-src 'unsafe-inline'"><style>
html{background:white;color:${paper.text}}
body{margin:0;padding:${padY} ${padX};min-height:calc(100vh - 2 * ${padY});font:${paper.font}/1.55 ${paper.family};outline:none;overflow-wrap:break-word}
body:empty::before{content:attr(data-placeholder);color:${paper.hint}}
img{max-width:100%;height:auto}
p{margin:0 0 .6em}
blockquote{margin:0 0 0 .8ex;padding-left:1ex;border-left:${paper.stroke} solid ${paper.quote}}
.lm-signature{color:${paper.muted}}
a{color:${paper.link}}
</style></head><body></body></html>`;

	function emit() {
		clearTimeout(timer);
		timer = setTimeout(() => {
			if (doc) onchange(doc.body.innerHTML);
		}, 120);
	}

	function updateStates() {
		if (!doc) return;
		const q = (c: string) => {
			try {
				return doc!.queryCommandState(c);
			} catch {
				return false;
			}
		};
		states = {
			bold: q("bold"),
			italic: q("italic"),
			underline: q("underline"),
			strikeThrough: q("strikeThrough"),
			insertUnorderedList: q("insertUnorderedList"),
			insertOrderedList: q("insertOrderedList"),
		};
	}

	function load() {
		doc = frame?.contentDocument ?? null;
		if (!doc) return;
		doc.body.contentEditable = "true";
		doc.body.spellcheck = true;
		doc.body.dataset.placeholder = placeholder;
		try {
			doc.execCommand("styleWithCSS", false, "false");
			doc.execCommand("defaultParagraphSeparator", false, "p");
		} catch {
			// Older engines keep their defaults.
		}
		doc.body.innerHTML = html;
		loadedVersion = version;
		doc.addEventListener("input", () => {
			emit();
			updateStates();
		});
		doc.addEventListener("selectionchange", updateStates);
		doc.addEventListener("keydown", (e) => onkeydown?.(e));
		doc.addEventListener("paste", onPaste);
		doc.addEventListener("drop", onDrop);
		doc.addEventListener("click", (e) => {
			// Links stay editable: never follow them inside the editor.
			if ((e.target as Element)?.closest?.("a")) e.preventDefault();
		});
	}

	$effect(() => {
		// Reload the body when it was replaced from outside (identity, format).
		// Read the props first: `doc` is not reactive, so a first run before
		// the frame loads would otherwise leave this effect without dependencies.
		const next = { version, html };
		if (doc && next.version !== loadedVersion) {
			doc.body.innerHTML = next.html;
			loadedVersion = next.version;
		}
	});

	$effect(() => () => {
		clearTimeout(timer);
		if (doc) onchange(doc.body.innerHTML);
	});

	function exec(command: string, value?: string) {
		if (!doc) return;
		frame?.contentWindow?.focus();
		doc.execCommand(command, false, value);
		emit();
		updateStates();
	}

	/** Hands the current body over now, without waiting for the debounce. */
	export function flush() {
		clearTimeout(timer);
		if (doc) onchange(doc.body.innerHTML);
	}

	export function focus() {
		frame?.contentWindow?.focus();
		doc?.body.focus();
	}

	export function insertHtml(fragment: string) {
		exec("insertHTML", fragment);
	}

	async function link() {
		const selection = frame?.contentWindow?.getSelection();
		const range = selection && selection.rangeCount ? selection.getRangeAt(0).cloneRange() : null;
		const url = await prompt({ title: t("editor.link"), label: t("editor.linkUrl"), placeholder: "https://", confirmLabel: t("editor.insert") });
		if (!url) return;
		const href = /^(https?:|mailto:|tel:)/i.test(url) ? url : url.includes("@") ? `mailto:${url}` : `https://${url}`;
		frame?.contentWindow?.focus();
		if (range && selection) {
			selection.removeAllRanges();
			selection.addRange(range);
		}
		if (range && !range.collapsed) exec("createLink", href);
		else exec("insertHTML", `<a href="${href.replace(/"/g, "&quot;")}">${href.replace(/</g, "&lt;")}</a>`);
	}

	function readImage(file: File): Promise<string> {
		return new Promise((resolve, reject) => {
			const reader = new FileReader();
			reader.onload = () => resolve(String(reader.result));
			reader.onerror = () => reject(reader.error);
			reader.readAsDataURL(file);
		});
	}

	async function insertImages(files: File[]) {
		for (const f of files.filter((f) => f.type.startsWith("image/"))) {
			const url = await readImage(f);
			exec("insertHTML", `<img src="${url}" alt="">`);
		}
	}

	function onPaste(e: ClipboardEvent) {
		const files = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith("image/"));
		if (files.length) {
			e.preventDefault();
			void insertImages(files);
		}
	}

	function onDrop(e: DragEvent) {
		const files = Array.from(e.dataTransfer?.files ?? []).filter((f) => f.type.startsWith("image/"));
		if (files.length) {
			e.preventDefault();
			void insertImages(files);
		}
	}

	const tools = $derived([
		{ cmd: "bold", icon: Bold, label: t("editor.bold") },
		{ cmd: "italic", icon: Italic, label: t("editor.italic") },
		{ cmd: "underline", icon: Underline, label: t("editor.underline") },
		{ cmd: "strikeThrough", icon: Strikethrough, label: t("editor.strike") },
	]);
	const lists = $derived([
		{ cmd: "insertUnorderedList", icon: List, label: t("editor.bullets") },
		{ cmd: "insertOrderedList", icon: ListOrdered, label: t("editor.numbers") },
		{ cmd: "outdent", icon: IndentDecrease, label: t("editor.outdent") },
		{ cmd: "indent", icon: IndentIncrease, label: t("editor.indent") },
	]);
	const aligns = $derived([
		{ cmd: "justifyLeft", icon: AlignLeft, label: t("editor.alignLeft") },
		{ cmd: "justifyCenter", icon: AlignCenter, label: t("editor.alignCenter") },
		{ cmd: "justifyRight", icon: AlignRight, label: t("editor.alignRight") },
	]);
	const sizes = $derived([
		{ value: "2", label: t("editor.small") },
		{ value: "3", label: t("editor.normal") },
		{ value: "5", label: t("editor.large") },
		{ value: "6", label: t("editor.huge") },
	]);
</script>

<div class="lm-editor">
	<div class="lm-editor__bar" role="toolbar" aria-label={t("editor.toolbar")}>
		{#each tools as tool (tool.cmd)}
			<Tooltip content={tool.label}>
				<button type="button" class="lm-tool" class:lm-tool--on={states[tool.cmd]} aria-label={tool.label} aria-pressed={!!states[tool.cmd]} onmousedown={(e) => e.preventDefault()} onclick={() => exec(tool.cmd)}><tool.icon size={15} /></button>
			</Tooltip>
		{/each}
		<span class="lm-editor__sep"></span>
		<Popover bind:open={sizesOpen} side="bottom" align="start">
			{#snippet triggerChild(props)}
				<button {...props} type="button" class="lm-tool" aria-label={t("editor.size")} title={t("editor.size")} onmousedown={(e) => e.preventDefault()}><Type size={15} /></button>
			{/snippet}
			<div class="lm-editor__menu">
				{#each sizes as s (s.value)}
					<button type="button" onmousedown={(e) => e.preventDefault()} onclick={() => ((sizesOpen = false), exec("fontSize", s.value))}>{s.label}</button>
				{/each}
			</div>
		</Popover>
		<Popover bind:open={colorsOpen} side="bottom" align="start">
			{#snippet triggerChild(props)}
				<button {...props} type="button" class="lm-tool" aria-label={t("editor.color")} title={t("editor.color")} onmousedown={(e) => e.preventDefault()}><Palette size={15} /></button>
			{/snippet}
			<div class="lm-editor__colors">
				{#each COLORS as c (c)}
					<button type="button" class="lm-swatch" style:background={c} aria-label={c} onmousedown={(e) => e.preventDefault()} onclick={() => ((colorsOpen = false), exec("foreColor", c))}></button>
				{/each}
			</div>
		</Popover>
		<span class="lm-editor__sep"></span>
		{#each lists as tool (tool.cmd)}
			<Tooltip content={tool.label}>
				<button type="button" class="lm-tool" class:lm-tool--on={states[tool.cmd]} aria-label={tool.label} onmousedown={(e) => e.preventDefault()} onclick={() => exec(tool.cmd)}><tool.icon size={15} /></button>
			</Tooltip>
		{/each}
		<Tooltip content={t("editor.quote")}>
			<button type="button" class="lm-tool" aria-label={t("editor.quote")} onmousedown={(e) => e.preventDefault()} onclick={() => exec("formatBlock", "blockquote")}><Quote size={15} /></button>
		</Tooltip>
		<span class="lm-editor__sep lm-editor__sep--wide"></span>
		{#each aligns as tool (tool.cmd)}
			<Tooltip content={tool.label}>
				<button type="button" class="lm-tool lm-tool--wide" aria-label={tool.label} onmousedown={(e) => e.preventDefault()} onclick={() => exec(tool.cmd)}><tool.icon size={15} /></button>
			</Tooltip>
		{/each}
		<span class="lm-editor__sep"></span>
		<Tooltip content={t("editor.link")}>
			<button type="button" class="lm-tool" aria-label={t("editor.link")} onmousedown={(e) => e.preventDefault()} onclick={link}><Link size={15} /></button>
		</Tooltip>
		<Tooltip content={t("editor.image")}>
			<button type="button" class="lm-tool" aria-label={t("editor.image")} onmousedown={(e) => e.preventDefault()} onclick={() => fileInput?.click()}><Image size={15} /></button>
		</Tooltip>
		<Tooltip content={t("editor.clear")}>
			<button type="button" class="lm-tool" aria-label={t("editor.clear")} onmousedown={(e) => e.preventDefault()} onclick={() => exec("removeFormat")}><RemoveFormatting size={15} /></button>
		</Tooltip>
		<input
			bind:this={fileInput}
			type="file"
			accept="image/*"
			multiple
			hidden
			onchange={(e) => {
				const input = e.currentTarget as HTMLInputElement;
				void insertImages(Array.from(input.files ?? []));
				input.value = "";
			}}
		/>
	</div>
	<iframe bind:this={frame} class="lm-editor__frame" title={t("editor.body")} srcdoc={shell} sandbox="allow-same-origin" style:min-height="{minHeight}px" onload={load}></iframe>
</div>

<style>
	.lm-editor {
		flex: 1;
		min-height: 0;
		display: flex;
		flex-direction: column;
		border: 1px solid var(--ca-outline-default);
	}
	.lm-editor__bar {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 1px;
		padding: var(--ca-space-1) var(--ca-space-1-5);
		border-bottom: 1px solid var(--ca-outline-subtle);
		background: var(--ca-surface-sunken);
	}
	.lm-tool {
		width: var(--lm-tool-wide);
		height: var(--lm-tool);
		display: grid;
		place-items: center;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		cursor: pointer;
	}
	.lm-tool:hover {
		background: var(--lm-row-hover);
		color: var(--ca-text-primary);
	}
	.lm-tool--on {
		background: var(--lm-row-open);
		color: var(--ca-text-primary);
	}
	.lm-editor__sep {
		width: 1px;
		height: var(--lm-separator-h);
		margin: 0 var(--ca-space-1-5);
		background: var(--ca-outline-subtle);
	}
	@media (max-width: 760px) {
		.lm-tool--wide,
		.lm-editor__sep--wide {
			display: none;
		}
	}
	.lm-editor__frame {
		flex: 1;
		width: 100%;
		border: 0;
		background: white;
	}
	.lm-editor__menu {
		display: flex;
		flex-direction: column;
		min-width: var(--lm-menu-min-w);
	}
	.lm-editor__menu button {
		padding: var(--ca-space-2) var(--ca-space-3-5);
		border: 0;
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.lm-editor__menu button:hover {
		background: var(--lm-row-hover);
	}
	.lm-editor__colors {
		display: grid;
		grid-template-columns: repeat(5, var(--lm-swatch));
		gap: var(--ca-space-2);
	}
	.lm-swatch {
		width: var(--lm-swatch);
		height: var(--lm-swatch);
		border: 1px solid var(--ca-outline-default);
		cursor: pointer;
	}
</style>
