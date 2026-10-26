<script lang="ts">
	import { Drawer } from "@codeatlasdev/ui";
	import { PenLine } from "@lucide/svelte";
	import { app } from "../../state/app.svelte";
	import { compose } from "../../state/compose.svelte";
	import { mail } from "../../state/mail.svelte";
	import { t } from "../../i18n/index.svelte";
	import FolderPane from "./FolderPane.svelte";
	import MessageList from "./MessageList.svelte";
	import MessageView from "./MessageView.svelte";
	import ComposeView from "../compose/ComposeView.svelte";
	import MoveDialog from "./MoveDialog.svelte";

	const SIDE_KEY = "lowmail.folders.width";
	const LIST_KEY = "lowmail.list.width";
	const LIST_H_KEY = "lowmail.list.height";

	function read(key: string, fallback: number) {
		try {
			return Number(localStorage.getItem(key)) || fallback;
		} catch {
			return fallback;
		}
	}

	let sideWidth = $state(read(SIDE_KEY, 240));
	let listWidth = $state(read(LIST_KEY, 440));
	let listHeight = $state(read(LIST_H_KEY, 320));
	let width = $state(1200);
	let foldersOpen = $state(false);
	let moving = $state(false);

	const narrow = $derived(width < 900);
	const phone = $derived(width < 680);
	const layout = $derived(phone ? "list" : app.settings.layout);
	/** On phones and in list layout the reader replaces the list. */
	const readerFull = $derived(layout === "list" && mail.open !== null);

	function drag(e: PointerEvent, apply: (ev: PointerEvent) => void, save: () => void) {
		e.preventDefault();
		const target = e.currentTarget as HTMLElement;
		target.setPointerCapture(e.pointerId);
		const up = () => {
			target.removeEventListener("pointermove", apply);
			target.removeEventListener("pointerup", up);
			save();
		};
		target.addEventListener("pointermove", apply);
		target.addEventListener("pointerup", up);
	}

	let viewEl: HTMLElement | undefined = $state();
	let listEl: HTMLElement | undefined = $state();

	function resizeSide(e: PointerEvent) {
		const left = viewEl?.getBoundingClientRect().left ?? 0;
		drag(e, (ev) => (sideWidth = Math.min(420, Math.max(170, ev.clientX - left))), () => localStorage.setItem(SIDE_KEY, String(sideWidth)));
	}

	function resizeList(e: PointerEvent) {
		const left = listEl?.getBoundingClientRect().left ?? 0;
		drag(e, (ev) => (listWidth = Math.min(900, Math.max(300, ev.clientX - left))), () => localStorage.setItem(LIST_KEY, String(listWidth)));
	}

	function resizeListHeight(e: PointerEvent) {
		const top = listEl?.getBoundingClientRect().top ?? 0;
		drag(e, (ev) => (listHeight = Math.min(900, Math.max(140, ev.clientY - top))), () => localStorage.setItem(LIST_H_KEY, String(listHeight)));
	}

	$effect(() => {
		const onMove = () => (moving = true);
		window.addEventListener("lm:move", onMove);
		return () => window.removeEventListener("lm:move", onMove);
	});
</script>

<div class="lm-mail" bind:clientWidth={width} bind:this={viewEl}>
	{#if narrow}
		<Drawer bind:open={foldersOpen} side="left" title={t("folders.title")} width="290px" class="lm-drawer">
			<FolderPane onnavigate={() => (foldersOpen = false)} />
		</Drawer>
	{:else}
		<aside class="lm-mail__side" style:width="{sideWidth}px">
			<FolderPane />
		</aside>
		<div class="lm-resizer" role="separator" aria-orientation="vertical" aria-label={t("layout.resize")} onpointerdown={resizeSide}></div>
	{/if}

	<section class="lm-mail__main" bind:this={listEl}>
		{#if compose.visible && compose.active}
			<ComposeView />
		{:else if layout === "columns"}
			<div class="lm-mail__columns">
				<div class="lm-mail__list" style:width="{listWidth}px">
					<MessageList {narrow} onfolders={() => (foldersOpen = true)} compact />
				</div>
				<div class="lm-resizer" role="separator" aria-orientation="vertical" aria-label={t("layout.resize")} onpointerdown={resizeList}></div>
				<div class="lm-mail__reader"><MessageView /></div>
			</div>
		{:else if layout === "rows"}
			<div class="lm-mail__rows">
				<div class="lm-mail__list" style:height="{listHeight}px">
					<MessageList {narrow} onfolders={() => (foldersOpen = true)} />
				</div>
				<div class="lm-resizer lm-resizer--rows" role="separator" aria-orientation="horizontal" aria-label={t("layout.resize")} onpointerdown={resizeListHeight}></div>
				<div class="lm-mail__reader"><MessageView /></div>
			</div>
		{:else if readerFull}
			<div class="lm-mail__reader"><MessageView back /></div>
		{:else}
			<div class="lm-mail__list lm-mail__list--full"><MessageList {narrow} onfolders={() => (foldersOpen = true)} compact={phone} /></div>
		{/if}

		{#if compose.active && !compose.visible}
			<button type="button" class="lm-compose-chip" onclick={() => compose.show()}>
				<PenLine size={15} />
				<span class="lm-ellipsis">{compose.draft?.subject || t("compose.newMessage")}</span>
			</button>
		{/if}
	</section>
</div>

<MoveDialog bind:open={moving} />

<style>
	.lm-mail {
		flex: 1;
		min-width: 0;
		display: flex;
	}
	.lm-mail__side {
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		background: var(--ca-surface-sunken);
		min-height: 0;
	}
	.lm-mail__main {
		position: relative;
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-mail__columns,
	.lm-mail__rows {
		flex: 1;
		min-height: 0;
		display: flex;
	}
	.lm-mail__rows {
		flex-direction: column;
	}
	.lm-mail__list {
		flex-shrink: 0;
		min-height: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-mail__list--full {
		flex: 1;
	}
	.lm-mail__reader {
		flex: 1;
		min-width: 0;
		min-height: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-compose-chip {
		position: absolute;
		right: var(--ca-space-5);
		bottom: var(--ca-space-5);
		z-index: 5;
		display: flex;
		align-items: center;
		gap: var(--ca-space-2-5);
		max-width: var(--lm-popover-w);
		padding: var(--ca-space-3-5) var(--ca-space-4-5);
		border: 1px solid var(--ca-outline-strong);
		background: var(--ca-surface-raised, var(--ca-surface-base));
		color: var(--ca-text-primary);
		font: inherit;
		font-weight: 600;
		cursor: pointer;
		box-shadow: var(--lm-lift) var(--lm-lift) 0 var(--lm-signal);
	}
</style>
