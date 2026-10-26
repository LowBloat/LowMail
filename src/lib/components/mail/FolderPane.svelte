<script lang="ts">
	import { DropdownMenu, DropdownMenuItem, DropdownMenuSeparator, Progress, toast } from "@codeatlasdev/ui";
	import { Archive, ChevronDown, ChevronRight, File, Folder, FolderPlus, Inbox, OctagonAlert, RefreshCw, Send, Trash2 } from "@lucide/svelte";
	import { call } from "../../api";
	import { app } from "../../state/app.svelte";
	import { mail, parseKey } from "../../state/mail.svelte";
	import { confirm, prompt } from "../../state/dialogs.svelte";
	import { errorText, t } from "../../i18n/index.svelte";
	import { folderLabel, size } from "../../format";
	import type { Folder as FolderT, FolderRole, Quota } from "../../types";

	let { onnavigate }: { onnavigate?: () => void } = $props();

	const COLLAPSED_KEY = "lowmail.folders.collapsed";
	let collapsed = $state<string[]>(readCollapsed());
	let menu = $state<{ folder: FolderT; x: number; y: number } | null>(null);
	let dropTarget = $state<string | null>(null);
	let quota = $state<Quota | null>(null);

	function readCollapsed(): string[] {
		try {
			return JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]");
		} catch {
			return [];
		}
	}

	function toggleCollapse(path: string) {
		collapsed = collapsed.includes(path) ? collapsed.filter((p) => p !== path) : [...collapsed, path];
		localStorage.setItem(COLLAPSED_KEY, JSON.stringify(collapsed));
	}

	const icons: Record<FolderRole, typeof Inbox> = { inbox: Inbox, drafts: File, sent: Send, archive: Archive, junk: OctagonAlert, trash: Trash2 };

	const visible = $derived.by(() => {
		const shown = mail.folders.filter((f) => f.subscribed || f.role);
		const paths = new Set(shown.map((f) => f.path));
		const hasChildren = new Set(shown.filter((f) => f.parent && paths.has(f.parent)).map((f) => f.parent!));
		const hidden = (f: FolderT): boolean => {
			let p = f.parent;
			while (p) {
				if (collapsed.includes(p)) return true;
				p = mail.folders.find((x) => x.path === p)?.parent ?? null;
			}
			return false;
		};
		return shown.filter((f) => !hidden(f)).map((f) => ({ folder: f, children: hasChildren.has(f.path) }));
	});

	async function loadQuota() {
		try {
			quota = await call<Quota | null>("quota.get");
		} catch {
			quota = null;
		}
	}

	$effect(() => {
		if (app.signedIn) void loadQuota();
	});

	function open(f: FolderT) {
		if (!f.selectable) {
			toggleCollapse(f.path);
			return;
		}
		void mail.openFolder(f.path);
		onnavigate?.();
	}

	function onContext(e: MouseEvent, folder: FolderT) {
		e.preventDefault();
		menu = { folder, x: e.clientX, y: e.clientY };
	}

	async function newFolder(parent: FolderT | null) {
		const name = await prompt({ title: parent ? t("folders.newSub", { name: folderLabel(parent) }) : t("folders.new"), label: t("folders.name"), confirmLabel: t("action.create") });
		if (!name) return;
		try {
			await call("folders.create", { parent: parent?.path ?? null, name });
			await mail.loadFolders();
			toast.success(t("folders.created"));
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	async function rename(f: FolderT) {
		const name = await prompt({ title: t("folders.rename"), label: t("folders.name"), value: f.name, confirmLabel: t("action.rename") });
		if (!name || name === f.name) return;
		try {
			const res = await call<{ path: string }>("folders.rename", { path: f.path, name });
			const wasCurrent = mail.folder === f.path;
			await mail.loadFolders();
			if (wasCurrent) await mail.openFolder(res.path);
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	async function remove(f: FolderT) {
		const ok = await confirm({ title: t("folders.delete.title", { name: folderLabel(f) }), description: t("folders.delete.body"), confirmLabel: t("action.delete"), destructive: true });
		if (!ok) return;
		try {
			await call("folders.delete", { path: f.path });
			if (mail.folder === f.path || mail.folder.startsWith(f.path + (f.delimiter ?? "/"))) await mail.openFolder("INBOX");
			await mail.loadFolders();
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	async function empty(f: FolderT) {
		const ok = await confirm({ title: t("folders.empty.title", { name: folderLabel(f) }), description: t("folders.empty.body", { n: f.total }), confirmLabel: t("folders.empty.confirm"), destructive: true });
		if (!ok) return;
		try {
			await call("folders.empty", { path: f.path });
			if (mail.folder === f.path) await mail.loadList();
			await mail.refreshCounts([f.path]);
			void loadQuota();
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	async function markRead(f: FolderT) {
		try {
			await call("folders.markRead", { path: f.path });
			f.unread = 0;
			mail.updateTitle();
			if (mail.folder === f.path) mail.refresh();
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	// Messages dragged from the list.
	function onDragOver(e: DragEvent, f: FolderT) {
		if (!f.selectable || !e.dataTransfer?.types.includes("application/x-lowmail-keys")) return;
		e.preventDefault();
		e.dataTransfer.dropEffect = e.ctrlKey ? "copy" : "move";
		dropTarget = f.path;
	}

	function onDrop(e: DragEvent, f: FolderT) {
		dropTarget = null;
		const data = e.dataTransfer?.getData("application/x-lowmail-keys");
		if (!data) return;
		e.preventDefault();
		const keys: string[] = JSON.parse(data);
		if (keys.every((k) => parseKey(k).folder === f.path)) return;
		if (e.ctrlKey) void mail.copyTo(f.path, keys);
		else void mail.moveTo(f.path, keys);
	}

	const quotaPercent = $derived(quota && quota.limit > 0 ? Math.min(100, Math.round((quota.used / quota.limit) * 100)) : 0);
</script>

<div class="lm-folders">
	<div class="lm-bar lm-folders__bar">
		<span class="lm-bar__title">{t("folders.title")}</span>
		<span class="lm-spacer"></span>
		<button type="button" class="lm-folders__tool" title={t("action.refresh")} aria-label={t("action.refresh")} onclick={() => mail.refresh()}>
			<RefreshCw size={15} class={mail.listLoading ? "lm-spin" : ""} />
		</button>
		<button type="button" class="lm-folders__tool" title={t("folders.new")} aria-label={t("folders.new")} onclick={() => newFolder(null)}>
			<FolderPlus size={15} />
		</button>
	</div>
	<ul class="lm-folders__list" role="tree" aria-label={t("folders.title")}>
		{#each visible as { folder: f, children } (f.path)}
			{@const Icon = f.role ? icons[f.role] : Folder}
			<li role="treeitem" aria-selected={mail.folder === f.path && !mail.searchEverywhere} aria-expanded={children ? !collapsed.includes(f.path) : undefined}>
				<div
					class="lm-folder"
					class:lm-folder--active={mail.folder === f.path && !mail.searchEverywhere}
					class:lm-folder--drop={dropTarget === f.path}
					class:lm-folder--disabled={!f.selectable}
					style:padding-left="{8 + f.depth * 14}px"
					role="button"
					tabindex="0"
					onclick={() => open(f)}
					onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), open(f))}
					oncontextmenu={(e) => onContext(e, f)}
					ondragover={(e) => onDragOver(e, f)}
					ondragleave={() => dropTarget === f.path && (dropTarget = null)}
					ondrop={(e) => onDrop(e, f)}
				>
					{#if children}
						<button type="button" class="lm-folder__twisty" aria-label={collapsed.includes(f.path) ? t("folders.expand") : t("folders.collapse")} onclick={(e) => (e.stopPropagation(), toggleCollapse(f.path))}>
							{#if collapsed.includes(f.path)}<ChevronRight size={13} />{:else}<ChevronDown size={13} />{/if}
						</button>
					{:else}
						<span class="lm-folder__twisty"></span>
					{/if}
					<Icon size={16} />
					<span class="lm-folder__name lm-ellipsis">{folderLabel(f)}</span>
					{#if f.unread > 0 && f.role !== "sent" && f.role !== "trash"}
						<span class="lm-folder__count" class:lm-folder__count--strong={f.role === "inbox" || !f.role}>{f.unread}</span>
					{:else if f.role === "drafts" && f.total > 0}
						<span class="lm-folder__count">{f.total}</span>
					{/if}
				</div>
			</li>
		{/each}
	</ul>
	{#if quota && quota.limit > 0}
		<div class="lm-folders__quota" title={t("quota.title")}>
			<Progress value={quotaPercent} size="sm" label={t("quota.label", { used: size(quota.used), limit: size(quota.limit) })} />
			<span class="lm-hint">{t("quota.label", { used: size(quota.used), limit: size(quota.limit) })}</span>
		</div>
	{/if}
</div>

{#if menu}
	{@const f = menu.folder}
	<DropdownMenu open anchor={{ x: menu.x, y: menu.y }} onOpenChange={(o) => !o && (menu = null)}>
		{#if f.selectable}
			<DropdownMenuItem onclick={() => markRead(f)} disabled={f.unread === 0}>{t("folders.markRead")}</DropdownMenuItem>
		{/if}
		{#if f.canHaveChildren}
			<DropdownMenuItem onclick={() => newFolder(f)}>{t("folders.newSubShort")}</DropdownMenuItem>
		{/if}
		{#if !f.role}
			<DropdownMenuItem onclick={() => rename(f)}>{t("folders.rename")}</DropdownMenuItem>
		{/if}
		{#if f.role === "trash" || f.role === "junk"}
			<DropdownMenuSeparator />
			<DropdownMenuItem variant="destructive" onclick={() => empty(f)} disabled={f.total === 0}>{t("folders.empty.action")}</DropdownMenuItem>
		{/if}
		<!-- Special folders stay: the settings screen protects them the same way. -->
		{#if !f.role}
			<DropdownMenuSeparator />
			<DropdownMenuItem variant="destructive" onclick={() => remove(f)}>{t("folders.delete.action")}</DropdownMenuItem>
		{/if}
	</DropdownMenu>
{/if}

<style>
	.lm-folders {
		flex: 1;
		min-height: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-folders__bar {
		background: transparent;
	}
	.lm-folders__tool {
		width: var(--lm-tool);
		height: var(--lm-tool);
		display: grid;
		place-items: center;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		cursor: pointer;
	}
	.lm-folders__tool:hover {
		background: var(--lm-row-hover);
		color: var(--ca-text-primary);
	}
	.lm-folders__list {
		flex: 1;
		min-height: 0;
		overflow: auto;
		list-style: none;
		margin: 0;
		padding: var(--ca-space-2) 0;
	}
	.lm-folder {
		display: flex;
		align-items: center;
		gap: var(--ca-space-2-5);
		height: var(--lm-nav-row);
		padding-right: var(--ca-space-3-5);
		color: var(--ca-text-secondary);
		cursor: pointer;
		user-select: none;
	}
	.lm-folder:hover {
		background: var(--lm-row-hover);
		color: var(--ca-text-primary);
	}
	.lm-folder--active {
		background: var(--ca-surface-base);
		color: var(--ca-text-primary);
		font-weight: 600;
		box-shadow: inset var(--lm-accent) 0 0 var(--lm-signal);
	}
	.lm-folder--drop {
		background: var(--lm-row-open);
		outline: 1px dashed var(--ca-outline-strong);
		outline-offset: -2px;
	}
	.lm-folder--disabled {
		opacity: 0.75;
	}
	.lm-folder__twisty {
		width: var(--lm-icon);
		height: var(--lm-icon);
		flex-shrink: 0;
		display: grid;
		place-items: center;
		padding: 0;
		border: 0;
		background: transparent;
		color: inherit;
		cursor: pointer;
	}
	.lm-folder__name {
		flex: 1;
	}
	.lm-folder__count {
		font-size: var(--ca-font-size-body2);
		font-variant-numeric: tabular-nums;
		color: var(--ca-text-secondary);
	}
	.lm-folder__count--strong {
		font-weight: 700;
		color: var(--ca-text-primary);
	}
	.lm-folders__quota {
		display: flex;
		flex-direction: column;
		gap: var(--ca-space-1-5);
		padding: var(--ca-space-3-5) var(--ca-space-4) var(--ca-space-4);
		border-top: 1px solid var(--ca-outline-subtle);
	}
	:global(.lm-spin) {
		animation: lm-spin 0.9s linear infinite;
	}
	@keyframes lm-spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
