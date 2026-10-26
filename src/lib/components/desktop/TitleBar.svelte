<script lang="ts">
	import { onMount } from "svelte";
	import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuItem, DropdownMenuSeparator } from "@codeatlasdev/ui";
	import { getVersion } from "@tauri-apps/api/app";
	import { exit } from "@tauri-apps/plugin-process";
	import { toast } from "@codeatlasdev/ui";
	import { IS_MAC, isSeparator, setNativeAppMenu, shortcutLabel } from "../../app-menu";
	import { windowFrame as frame } from "../../state/window-frame.svelte";
	import { buildMenus } from "../../menus";
	import { app } from "../../state/app.svelte";
	import { openExternal } from "../../api";
	import { t } from "../../i18n/index.svelte";

	const title = "LowMail";
	const detail = $derived(app.account?.email ?? "");

	async function about() {
		const version = await getVersion().catch(() => "");
		toast.info(`LowMail ${version} · AGPL-3.0-only · LowBloat`);
	}

	const actions = {
		shortcuts: () => window.dispatchEvent(new CustomEvent("lm:shortcuts")),
		about: () => void about(),
		quit: () => void exit(0),
		docs: () => void openExternal("https://github.com/LowBloat/LowMail#readme"),
	};

	const menus = $derived(buildMenus(actions));

	$effect(() => {
		if (IS_MAC) void setNativeAppMenu(menus, { label: t("menu.about"), action: actions.about });
	});

	let openIndex = $state<number | null>(null);
	let triggers: HTMLElement[] = $state([]);
	// Alt pressed and released alone moves the focus to the menu bar, as on Windows.
	let altAlone = false;
	let focusBefore: HTMLElement | null = null;

	function modalOpen() {
		return !!document.querySelector('[role="dialog"][aria-modal="true"], dialog[open]');
	}

	function setOpen(index: number, open: boolean) {
		if (open && modalOpen()) return;
		if (open) openIndex = index;
		else if (openIndex === index) openIndex = null;
	}

	function focusMenuBar() {
		const active = document.activeElement;
		if (active instanceof HTMLElement && !triggers.includes(active)) focusBefore = active;
		triggers[0]?.focus();
	}

	function leaveMenuBar() {
		const target = focusBefore?.isConnected ? focusBefore : null;
		focusBefore = null;
		target?.focus();
	}

	function onTriggerKeydown(event: KeyboardEvent, index: number) {
		if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
			if (event.key === "Escape" && openIndex === null) leaveMenuBar();
			return;
		}
		event.preventDefault();
		const next = (index + (event.key === "ArrowRight" ? 1 : -1) + menus.length) % menus.length;
		triggers[next]?.focus();
	}

	function onWindowKeydown(event: KeyboardEvent) {
		altAlone = event.key === "Alt" && !event.repeat && !event.ctrlKey && !event.shiftKey && !event.metaKey;
		if (IS_MAC) return;
		if (event.key === "F10" && !event.shiftKey && !event.ctrlKey && !event.altKey && !modalOpen()) {
			event.preventDefault();
			focusMenuBar();
		} else if (openIndex !== null && (event.key === "ArrowLeft" || event.key === "ArrowRight")
			&& !(event.target as Element | null)?.closest?.('[aria-haspopup="menu"][aria-expanded="true"]:not([data-titlebar-menu])')) {
			// Left and right move between the menus of the bar, as in native menu bars.
			event.preventDefault();
			event.stopPropagation();
			const next = (openIndex + (event.key === "ArrowRight" ? 1 : -1) + menus.length) % menus.length;
			openIndex = next;
		}
	}

	function onWindowKeyup(event: KeyboardEvent) {
		if (event.key !== "Alt" || !altAlone || IS_MAC || modalOpen()) return;
		altAlone = false;
		event.preventDefault();
		if (triggers.includes(document.activeElement as HTMLElement)) leaveMenuBar();
		else focusMenuBar();
	}

	onMount(() => {
		void frame.start();
		return () => frame.stop();
	});
</script>

<svelte:window onkeydowncapture={onWindowKeydown} onkeyup={onWindowKeyup} />

<header class="lm-titlebar" class:lm-titlebar--mac={IS_MAC} class:lm-titlebar--inactive={!frame.focused} data-tauri-drag-region>
	{#if !IS_MAC}
		<img src="./favicon.svg" alt="" class="lm-titlebar__logo" draggable="false" data-tauri-drag-region />
		<div class="lm-titlebar__menus" role="menubar" aria-label={t("menu.label")}>
			{#each menus as menu, index (menu.id)}
				<DropdownMenu open={openIndex === index} onOpenChange={(open) => setOpen(index, open)} align="start" sideOffset={2}>
					{#snippet triggerChild(props)}
						<button
							{...props}
							bind:this={triggers[index]}
							type="button"
							class="lm-titlebar__menu"
							data-titlebar-menu
							tabindex={index === 0 ? 0 : -1}
							onpointerenter={() => { if (openIndex !== null && openIndex !== index) openIndex = index; }}
							onkeydown={(event) => onTriggerKeydown(event, index)}
						>{menu.label}</button>
					{/snippet}
					{#each menu.entries as entry, i (i)}
						{#if isSeparator(entry)}
							<DropdownMenuSeparator />
						{:else if entry.checked !== undefined}
							<DropdownMenuCheckboxItem checked={entry.checked} disabled={entry.disabled} closeOnSelect onCheckedChange={() => void entry.action()}>
								{entry.label}
							</DropdownMenuCheckboxItem>
						{:else}
							<DropdownMenuItem disabled={entry.disabled} onclick={() => void entry.action()}>
								<span class="lm-titlebar__item">{entry.label}{#if entry.shortcut}<span class="lm-titlebar__key">{shortcutLabel(entry.shortcut)}</span>{/if}</span>
							</DropdownMenuItem>
						{/if}
					{/each}
				</DropdownMenu>
			{/each}
		</div>
	{/if}

	<!-- Centered on the window, not on the space between menus and buttons. -->
	<div class="lm-titlebar__title">
		<span class="lm-titlebar__name">{title}</span>
		{#if detail}<span class="lm-titlebar__detail">— {detail}</span>{/if}
	</div>

	<div class="lm-titlebar__drag" data-tauri-drag-region></div>

	{#if !IS_MAC}
		<div class="lm-titlebar__buttons">
			<button type="button" class="lm-titlebar__button" onclick={frame.minimize} aria-label={t("window.minimize")} title={t("window.minimize")}>
				<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" /></svg>
			</button>
			<button type="button" class="lm-titlebar__button" onclick={frame.toggleMaximize}
				aria-label={frame.maximized ? t("window.restore") : t("window.maximize")} title={frame.maximized ? t("window.restore") : t("window.maximize")}>
				{#if frame.maximized}
					<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7z" /></svg>
				{:else}
					<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M.5.5h9v9h-9z" /></svg>
				{/if}
			</button>
			<button type="button" class="lm-titlebar__button lm-titlebar__button--close" onclick={frame.close} aria-label={t("window.close")} title={t("window.close")}>
				<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" /></svg>
			</button>
		</div>
	{/if}
</header>

<style>
	.lm-titlebar {
		position: relative;
		display: flex;
		align-items: stretch;
		flex-shrink: 0;
		height: 34px;
		border-bottom: 1px solid var(--ca-outline-subtle);
		background: var(--ca-surface-sunken);
		color: var(--ca-text-secondary);
		font-size: 13px;
		user-select: none;
	}
	/* Room for the native traffic lights drawn over the bar. */
	.lm-titlebar--mac {
		padding-left: 78px;
	}
	.lm-titlebar__logo {
		width: 16px;
		height: 16px;
		align-self: center;
		margin: 0 10px 0 12px;
	}
	.lm-titlebar__menus {
		display: flex;
		align-items: stretch;
	}
	.lm-titlebar__menu {
		margin: 4px 0;
		padding: 0 10px;
		border: 0;
		background: transparent;
		color: inherit;
		font: inherit;
		cursor: default;
		outline: none;
	}
	.lm-titlebar__menu:hover,
	.lm-titlebar__menu:focus-visible,
	.lm-titlebar__menu[aria-expanded="true"] {
		background: var(--ca-surface-raised, var(--ca-surface-base));
		color: var(--ca-text-primary);
	}
	.lm-titlebar__title {
		position: absolute;
		inset: 0 auto 0 50%;
		transform: translateX(-50%);
		display: flex;
		align-items: center;
		gap: 6px;
		max-width: 45%;
		min-width: 0;
		pointer-events: none;
	}
	.lm-titlebar__name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-weight: 600;
		color: var(--ca-text-primary);
	}
	.lm-titlebar--inactive .lm-titlebar__name {
		color: var(--ca-text-secondary);
	}
	.lm-titlebar__detail {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--ca-text-tertiary, var(--ca-text-secondary));
	}
	.lm-titlebar__drag {
		flex: 1;
	}
	.lm-titlebar__buttons {
		display: flex;
	}
	.lm-titlebar__button {
		width: 46px;
		display: grid;
		place-items: center;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		outline: none;
	}
	.lm-titlebar__button:hover,
	.lm-titlebar__button:focus-visible {
		background: var(--ca-surface-raised, var(--ca-surface-base));
		color: var(--ca-text-primary);
	}
	.lm-titlebar__button--close:hover,
	.lm-titlebar__button--close:focus-visible {
		background: #c42b1c;
		color: #fff;
	}
	.lm-titlebar__button svg {
		width: 10px;
		height: 10px;
		fill: none;
		stroke: currentColor;
		stroke-width: 1;
		shape-rendering: crispEdges;
	}
	.lm-titlebar__item {
		display: flex;
		align-items: center;
		gap: 24px;
		width: 100%;
	}
	.lm-titlebar__key {
		margin-left: auto;
		font-size: 11px;
		color: var(--ca-text-tertiary, var(--ca-text-secondary));
	}
</style>
