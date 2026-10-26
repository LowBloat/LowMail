<script lang="ts">
	import { Dialog } from "@codeatlasdev/ui";
	import { focusTextField } from "@codeatlasdev/ui/actions";
	import { Folder } from "@lucide/svelte";
	import { mail, parseKey } from "../../state/mail.svelte";
	import { t } from "../../i18n/index.svelte";
	import { folderLabel } from "../../format";

	let { open = $bindable(false) }: { open?: boolean } = $props();

	let copy = $state(false);
	let query = $state("");
	let active = $state(0);
	let input: HTMLInputElement | undefined = $state();

	$effect(() => {
		const onMove = (e: Event) => (copy = !!(e as CustomEvent).detail?.copy);
		window.addEventListener("lm:move", onMove);
		return () => window.removeEventListener("lm:move", onMove);
	});

	$effect(() => {
		if (open) {
			query = "";
			active = 0;
			if (!mail.targets.length) open = false;
			else requestAnimationFrame(() => input && focusTextField(input));
		}
	});

	const from = $derived(new Set(mail.targets.map((k) => parseKey(k).folder)));
	const options = $derived(
		mail.folders.filter((f) => f.selectable && !(from.size === 1 && from.has(f.path)) && (f.displayPath.toLowerCase().includes(query.toLowerCase()) || folderLabel(f).toLowerCase().includes(query.toLowerCase()))),
	);

	function choose(path: string) {
		open = false;
		if (copy) void mail.copyTo(path);
		else void mail.moveTo(path);
	}

	function onKey(e: KeyboardEvent) {
		if (e.key === "ArrowDown") (e.preventDefault(), (active = Math.min(options.length - 1, active + 1)));
		if (e.key === "ArrowUp") (e.preventDefault(), (active = Math.max(0, active - 1)));
		if (e.key === "Enter" && options[active]) (e.preventDefault(), choose(options[active].path));
	}
</script>

<Dialog bind:open title={copy ? t("move.copyTitle", { n: mail.targets.length }) : t("move.title", { n: mail.targets.length })} size="sm">
	<input bind:this={input} class="lm-input lm-move__filter" placeholder={t("move.filter")} bind:value={query} onkeydown={onKey} oninput={() => (active = 0)} />
	<ul class="lm-move" role="listbox" aria-label={t("folders.title")}>
		{#each options as f, i (f.path)}
			<li role="option" aria-selected={i === active}>
				<button type="button" class="lm-move__item" class:lm-move__item--active={i === active} style:padding-left="{10 + f.depth * 14}px" onclick={() => choose(f.path)} onmouseenter={() => (active = i)}>
					<Folder size={15} />
					<span class="lm-ellipsis">{folderLabel(f)}</span>
				</button>
			</li>
		{:else}
			<li class="lm-hint lm-move__none">{t("move.none")}</li>
		{/each}
	</ul>
</Dialog>

<style>
	.lm-move__filter {
		width: 100%;
		box-sizing: border-box;
		margin-bottom: 8px;
	}
	.lm-move {
		list-style: none;
		margin: 0;
		padding: 0;
		max-height: 50vh;
		overflow: auto;
		border: 1px solid var(--ca-outline-subtle);
	}
	.lm-move__item {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 8px;
		height: 34px;
		border: 0;
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.lm-move__item--active {
		background: var(--lm-row-open);
	}
	.lm-move__none {
		padding: 12px;
	}
</style>
