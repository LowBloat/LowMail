<script lang="ts">
	import { Button, Checkbox, DropdownMenu, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, EmptyState, IconButton, Popover, Select, Spinner, Switch, Tooltip } from "@codeatlasdev/ui";
	import { Archive, ArrowDownUp, Flag, Forward, Inbox, Mail, MailOpen, Menu, OctagonAlert, Paperclip, Reply, Search, SearchX, SlidersHorizontal, Trash2, X, FolderInput, CircleAlert } from "@lucide/svelte";
	import { app } from "../../state/app.svelte";
	import { compose } from "../../state/compose.svelte";
	import { emptyFilter, filterActive, keyOf, mail, parseKey, type RowKey } from "../../state/mail.svelte";
	import { saveMessage } from "../../api";
	import { t } from "../../i18n/index.svelte";
	import { displayName, folderLabel, listDate, size as fmtSize } from "../../format";
	import type { Filter, MessageSummary, SearchScope, SortField } from "../../types";

	let { narrow = false, compact = false, onfolders }: { narrow?: boolean; compact?: boolean; onfolders?: () => void } = $props();

	let scroller: HTMLDivElement | undefined = $state();
	let scrollTop = $state(0);
	let viewport = $state(400);
	let searchText = $state("");
	let searchInput: HTMLInputElement | undefined = $state();
	let filterOpen = $state(false);
	let draft = $state<Filter>(emptyFilter());
	let everywhere = $state(false);
	let menu = $state<{ key: RowKey; x: number; y: number } | null>(null);

	const rowHeight = $derived(compact ? (app.settings.density === "comfortable" ? 68 : 58) : app.settings.density === "comfortable" ? 42 : 34);
	const first = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - 6));
	const last = $derived(Math.min(mail.keys.length, Math.ceil((scrollTop + viewport) / rowHeight) + 6));
	const slice = $derived(mail.keys.slice(first, last));
	const selecting = $derived(mail.selected.size > 0);
	const allSelected = $derived(mail.keys.length > 0 && mail.selected.size === mail.keys.length);
	const folder = $derived(mail.current);
	const sentLike = $derived(folder?.role === "sent" || folder?.role === "drafts");
	const searching = $derived(filterActive(mail.filter) || mail.searchEverywhere);
	const title = $derived(mail.searchEverywhere ? t("search.everywhere") : folder ? folderLabel(folder) : mail.folder);

	$effect(() => {
		void mail.ensureRows(first, last);
	});

	// Keep the keyboard cursor in view.
	$effect(() => {
		const key = mail.focused;
		if (!key || !scroller) return;
		const i = mail.keys.indexOf(key);
		if (i < 0) return;
		const top = i * rowHeight;
		if (top < scroller.scrollTop) scroller.scrollTop = top;
		else if (top + rowHeight > scroller.scrollTop + viewport) scroller.scrollTop = top + rowHeight - viewport;
	});

	// A new folder starts at the top.
	$effect(() => {
		void mail.folder;
		void mail.searchEverywhere;
		if (scroller) scroller.scrollTop = 0;
		scrollTop = 0;
	});

	$effect(() => {
		searchText = mail.filter.text;
	});

	$effect(() => {
		const focus = () => searchInput?.focus();
		window.addEventListener("lm:search", focus);
		return () => window.removeEventListener("lm:search", focus);
	});

	const quickFilter = $derived(mail.filter.unread ? "unread" : mail.filter.flagged ? "flagged" : mail.filter.unanswered ? "unanswered" : "all");

	function setQuickFilter(value: string) {
		const f = { ...$state.snapshot(mail.filter), unread: value === "unread", flagged: value === "flagged", unanswered: value === "unanswered" };
		mail.search(f, mail.searchEverywhere);
	}

	function runSearch() {
		mail.search({ ...$state.snapshot(mail.filter), text: searchText }, mail.searchEverywhere);
	}

	function onSearchKey(e: KeyboardEvent) {
		if (e.key === "Enter") runSearch();
		if (e.key === "Escape") {
			searchText = "";
			if (searching) mail.clearSearch();
			searchInput?.blur();
		}
	}

	function openFilters() {
		draft = { ...$state.snapshot(mail.filter), text: searchText };
		everywhere = mail.searchEverywhere;
		filterOpen = true;
	}

	function applyFilters() {
		filterOpen = false;
		searchText = draft.text;
		mail.search($state.snapshot(draft), everywhere);
	}

	function sender(m: MessageSummary) {
		if (sentLike) {
			const to = m.to.map((a) => displayName(a)).join(", ");
			return to ? `${t("list.to")} ${to}` : t("list.noRecipients");
		}
		return displayName(m.from[0]) || t("list.unknownSender");
	}

	function onRowClick(e: MouseEvent, key: RowKey) {
		mail.click(key, e);
	}

	function onDragStart(e: DragEvent, key: RowKey) {
		if (!e.dataTransfer) return;
		const keys = mail.selected.has(key) ? mail.keys.filter((k) => mail.selected.has(k)) : [key];
		e.dataTransfer.setData("application/x-lowmail-keys", JSON.stringify(keys));
		e.dataTransfer.effectAllowed = "copyMove";
		const ghost = document.createElement("div");
		ghost.textContent = t("list.dragging", { n: keys.length });
		ghost.className = "lm-drag-ghost";
		document.body.appendChild(ghost);
		e.dataTransfer.setDragImage(ghost, -10, -10);
		setTimeout(() => ghost.remove(), 0);
	}

	function onContext(e: MouseEvent, key: RowKey) {
		e.preventDefault();
		if (!mail.selected.has(key)) {
			mail.selected.clear();
			mail.focused = key;
		}
		menu = { key, x: e.clientX, y: e.clientY };
	}

	const sortOptions: { value: SortField; label: string }[] = $derived([
		{ value: "date", label: t("sort.date") },
		{ value: "arrival", label: t("sort.arrival") },
		{ value: "from", label: t("sort.from") },
		{ value: "to", label: t("sort.to") },
		{ value: "subject", label: t("sort.subject") },
		{ value: "size", label: t("sort.size") },
	]);

	const scopes: { value: SearchScope; label: string }[] = $derived([
		{ value: "headers", label: t("search.scope.headers") },
		{ value: "from", label: t("search.scope.from") },
		{ value: "to", label: t("search.scope.to") },
		{ value: "subject", label: t("search.scope.subject") },
		{ value: "body", label: t("search.scope.body") },
		{ value: "everything", label: t("search.scope.everything") },
	]);

	function folderName(path: string) {
		const f = mail.folders.find((x) => x.path === path);
		return f ? folderLabel(f) : path;
	}
</script>

<div class="lm-pane lm-list">
	<div class="lm-bar">
		{#if narrow}
			<IconButton variant="text" size="sm" aria-label={t("folders.title")} onclick={onfolders}><Menu size={17} /></IconButton>
		{/if}
		{#if selecting}
			<Checkbox checked={allSelected} indeterminate={!allSelected} aria-label={t("list.selectAll")} onchange={(c) => (c ? mail.selectAll() : mail.selectNone())} />
			<span class="lm-bar__title">{t("list.selected", { n: mail.selected.size })}</span>
			<span class="lm-spacer"></span>
			<Tooltip content={t("action.toggleRead")}><IconButton variant="text" size="sm" aria-label={t("action.toggleRead")} onclick={() => mail.toggleRead()}><MailOpen size={16} /></IconButton></Tooltip>
			<Tooltip content={t("action.flag")}><IconButton variant="text" size="sm" aria-label={t("action.flag")} onclick={() => mail.toggleFlag()}><Flag size={16} /></IconButton></Tooltip>
			<Tooltip content={t("action.move")}><IconButton variant="text" size="sm" aria-label={t("action.move")} onclick={() => window.dispatchEvent(new CustomEvent("lm:move"))}><FolderInput size={16} /></IconButton></Tooltip>
			<Tooltip content={t("action.archive")}><IconButton variant="text" size="sm" aria-label={t("action.archive")} onclick={() => mail.archive()}><Archive size={16} /></IconButton></Tooltip>
			<Tooltip content={t("action.junk")}><IconButton variant="text" size="sm" aria-label={t("action.junk")} onclick={() => mail.junk(folder?.role !== "junk")}><OctagonAlert size={16} /></IconButton></Tooltip>
			<Tooltip content={t("action.delete")}><IconButton variant="text" size="sm" aria-label={t("action.delete")} onclick={() => mail.remove()}><Trash2 size={16} /></IconButton></Tooltip>
			<IconButton variant="text" size="sm" aria-label={t("list.clearSelection")} onclick={() => mail.selectNone()}><X size={16} /></IconButton>
		{:else}
			<span class="lm-bar__title" title={title}>{title}</span>
			{#if mail.total > 0}<span class="lm-hint lm-list__total">{mail.total.toLocaleString()}</span>{/if}
			<span class="lm-spacer"></span>
			<DropdownMenu align="end">
				{#snippet triggerChild(props)}
					<button {...props} type="button" class="lm-list__tool" aria-label={t("sort.label")} title={t("sort.label")}><ArrowDownUp size={15} /></button>
				{/snippet}
				<DropdownMenuRadioGroup value={mail.sort.field} onValueChange={(v: string) => mail.setSort({ field: v as SortField, descending: mail.sort.descending })}>
					{#each sortOptions as o (o.value)}
						<DropdownMenuRadioItem value={o.value}>{o.label}</DropdownMenuRadioItem>
					{/each}
				</DropdownMenuRadioGroup>
				<DropdownMenuSeparator />
				<DropdownMenuRadioGroup value={mail.sort.descending ? "desc" : "asc"} onValueChange={(v: string) => mail.setSort({ field: mail.sort.field, descending: v === "desc" })}>
					<DropdownMenuRadioItem value="desc">{t("sort.descending")}</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="asc">{t("sort.ascending")}</DropdownMenuRadioItem>
				</DropdownMenuRadioGroup>
				<DropdownMenuSeparator />
				<DropdownMenuLabel>{t("show.label")}</DropdownMenuLabel>
				<DropdownMenuRadioGroup value={quickFilter} onValueChange={(v: string) => setQuickFilter(v)}>
					<DropdownMenuRadioItem value="all">{t("show.all")}</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="unread">{t("show.unread")}</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="flagged">{t("show.flagged")}</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="unanswered">{t("show.unanswered")}</DropdownMenuRadioItem>
				</DropdownMenuRadioGroup>
			</DropdownMenu>
		{/if}
	</div>

	<div class="lm-list__search">
		<div class="lm-search">
			<Search size={15} />
			<input
				bind:this={searchInput}
				bind:value={searchText}
				type="search"
				placeholder={t("search.placeholder")}
				aria-label={t("search.placeholder")}
				onkeydown={onSearchKey}
			/>
			{#if searching}
				<button type="button" class="lm-list__tool" aria-label={t("search.clear")} title={t("search.clear")} onclick={() => ((searchText = ""), mail.clearSearch())}><X size={14} /></button>
			{/if}
			<Popover bind:open={filterOpen} side="bottom" align="end">
				{#snippet triggerChild(props)}
					<button {...props} type="button" class="lm-list__tool" class:lm-list__tool--on={searching} aria-label={t("search.filters")} title={t("search.filters")} onclick={openFilters}><SlidersHorizontal size={15} /></button>
				{/snippet}
				<form class="lm-filters" onsubmit={(e) => (e.preventDefault(), applyFilters())}>
					<label class="lm-filters__field">
						<span>{t("search.text")}</span>
						<input class="lm-input" type="search" bind:value={draft.text} />
					</label>
					<Select label={t("search.scope.label")} size="sm" value={draft.scope} options={scopes} onchange={(v) => (draft.scope = v as SearchScope)} />
					<div class="lm-filters__row">
						<label class="lm-filters__field"><span>{t("search.since")}</span><input class="lm-input" type="date" bind:value={draft.since} /></label>
						<label class="lm-filters__field"><span>{t("search.before")}</span><input class="lm-input" type="date" bind:value={draft.before} /></label>
					</div>
					<Checkbox label={t("search.unread")} bind:checked={draft.unread} />
					<Checkbox label={t("search.flagged")} bind:checked={draft.flagged} />
					<Checkbox label={t("search.answered")} bind:checked={draft.answered} />
					<Switch label={t("search.allFolders")} bind:checked={everywhere} />
					<div class="lm-filters__actions">
						<Button variant="ghost" size="sm" onclick={() => ((draft = emptyFilter()), (everywhere = false))}>{t("search.reset")}</Button>
						<Button variant="filled" size="sm" type="submit">{t("search.apply")}</Button>
					</div>
				</form>
			</Popover>
		</div>
	</div>

	<div
		class="lm-list__scroll"
		bind:this={scroller}
		bind:clientHeight={viewport}
		onscroll={() => (scrollTop = scroller?.scrollTop ?? 0)}
		role="listbox"
		aria-multiselectable="true"
		aria-label={title}
		tabindex="-1"
	>
		{#if mail.listError && !mail.keys.length}
			<EmptyState icon={CircleAlert} title={t("list.error")} description={mail.listError}>
				{#snippet actions()}<Button variant="outlined" size="sm" onclick={() => mail.loadList()}>{t("action.retry")}</Button>{/snippet}
			</EmptyState>
		{:else if mail.listLoading && !mail.keys.length}
			<div class="lm-list__loading"><Spinner size={22} label={t("app.loading")} /></div>
		{:else if !mail.keys.length}
			<EmptyState icon={searching ? SearchX : Inbox} title={searching ? t("list.noResults") : t("list.empty")} description={searching ? t("list.noResultsHint") : ""} />
		{:else}
			<div class="lm-list__space" style:height="{mail.keys.length * rowHeight}px">
				{#each slice as key, i (key)}
					{@const m = mail.rows.get(key)}
					{@const index = first + i}
					<!-- Rows are driven by the list's keyboard shortcuts (J/K, arrows, Enter). -->
					<!-- svelte-ignore a11y_click_events_have_key_events -->
					<div
						class="lm-row"
						class:lm-row--compact={compact}
						class:lm-row--unread={m && !m.flags.seen}
						class:lm-row--selected={mail.selected.has(key)}
						class:lm-row--open={mail.open === key}
						class:lm-row--focus={mail.focused === key}
						style:top="{index * rowHeight}px"
						style:height="{rowHeight}px"
						role="option"
						aria-selected={mail.selected.has(key) || mail.open === key}
						tabindex="-1"
						draggable="true"
						onclick={(e) => onRowClick(e, key)}
						ondblclick={() => m && mail.folderByRole("drafts")?.path !== m.folder && mail.openMessage(key)}
						oncontextmenu={(e) => onContext(e, key)}
						ondragstart={(e) => onDragStart(e, key)}
					>
						<span class="lm-row__check" class:lm-row__check--show={selecting}>
							<Checkbox checked={mail.selected.has(key)} aria-label={t("list.select")} onchange={() => mail.toggle(key)} />
						</span>
						{#if m}
							{#if compact}
								<div class="lm-row__body">
									<div class="lm-row__line">
										<span class="lm-row__from lm-ellipsis">{sender(m)}</span>
										{#if m.hasAttachments}<Paperclip size={13} class="lm-row__icon" />{/if}
										<span class="lm-row__date">{listDate(m.date)}</span>
									</div>
									<div class="lm-row__line">
										{#if m.flags.answered}<Reply size={13} class="lm-row__icon" />{:else if m.flags.forwarded}<Forward size={13} class="lm-row__icon" />{/if}
										{#if m.priority === "high"}<span class="lm-row__prio" title={t("message.highPriority")}>!</span>{/if}
										<span class="lm-row__subject lm-ellipsis">{m.subject || t("message.noSubject")}</span>
										{#if mail.searchEverywhere}<span class="lm-row__folder">{folderName(m.folder)}</span>{/if}
										<button type="button" class="lm-row__flag" class:lm-row__flag--on={m.flags.flagged} aria-label={m.flags.flagged ? t("action.unflag") : t("action.flag")} onclick={(e) => (e.stopPropagation(), mail.setFlags([key], { flagged: !m.flags.flagged }))}>
											<Flag size={13} />
										</button>
									</div>
								</div>
							{:else}
								<button type="button" class="lm-row__flag" class:lm-row__flag--on={m.flags.flagged} aria-label={m.flags.flagged ? t("action.unflag") : t("action.flag")} onclick={(e) => (e.stopPropagation(), mail.setFlags([key], { flagged: !m.flags.flagged }))}>
									<Flag size={13} />
								</button>
								<span class="lm-row__from lm-row__from--wide lm-ellipsis">{sender(m)}</span>
								<span class="lm-row__status">
									{#if m.flags.answered}<Reply size={13} class="lm-row__icon" />{:else if m.flags.forwarded}<Forward size={13} class="lm-row__icon" />{/if}
									{#if m.priority === "high"}<span class="lm-row__prio" title={t("message.highPriority")}>!</span>{/if}
								</span>
								<span class="lm-row__subject lm-ellipsis">{m.subject || t("message.noSubject")}</span>
								{#if mail.searchEverywhere}<span class="lm-row__folder">{folderName(m.folder)}</span>{/if}
								{#if m.hasAttachments}<Paperclip size={13} class="lm-row__icon" />{/if}
								<span class="lm-row__size lm-hint">{fmtSize(m.size)}</span>
								<span class="lm-row__date">{listDate(m.date)}</span>
							{/if}
						{:else}
							<div class="lm-row__body lm-row__placeholder" aria-hidden="true"><span></span><span></span></div>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	</div>
</div>

{#if menu}
	{@const key = menu.key}
	{@const m = mail.rows.get(key)}
	{@const ref = parseKey(key)}
	{@const many = mail.selected.size > 1}
	<DropdownMenu open anchor={{ x: menu.x, y: menu.y }} onOpenChange={(o) => !o && (menu = null)}>
		{#if !many}
			<DropdownMenuItem onclick={() => mail.openMessage(key)}>{t("action.open")}</DropdownMenuItem>
			<DropdownMenuSeparator />
			<DropdownMenuItem onclick={() => compose.open("reply", ref)}>{#snippet icon()}<Reply size={14} />{/snippet}{t("action.reply")}</DropdownMenuItem>
			<DropdownMenuItem onclick={() => compose.open("replyAll", ref)}>{t("action.replyAll")}</DropdownMenuItem>
			<DropdownMenuItem onclick={() => compose.open("forward", ref)}>{#snippet icon()}<Forward size={14} />{/snippet}{t("action.forward")}</DropdownMenuItem>
			<DropdownMenuSeparator />
		{/if}
		<DropdownMenuItem onclick={() => mail.toggleRead()}>{#snippet icon()}{#if m?.flags.seen}<Mail size={14} />{:else}<MailOpen size={14} />{/if}{/snippet}{m?.flags.seen && !many ? t("action.markUnread") : t("action.toggleRead")}</DropdownMenuItem>
		<DropdownMenuItem onclick={() => mail.toggleFlag()}>{#snippet icon()}<Flag size={14} />{/snippet}{m?.flags.flagged && !many ? t("action.unflag") : t("action.flag")}</DropdownMenuItem>
		<DropdownMenuItem onclick={() => window.dispatchEvent(new CustomEvent("lm:move"))}>{#snippet icon()}<FolderInput size={14} />{/snippet}{t("action.moveTo")}</DropdownMenuItem>
		<DropdownMenuItem onclick={() => window.dispatchEvent(new CustomEvent("lm:move", { detail: { copy: true } }))}>{t("action.copyTo")}</DropdownMenuItem>
		<DropdownMenuItem onclick={() => mail.archive()}>{#snippet icon()}<Archive size={14} />{/snippet}{t("action.archive")}</DropdownMenuItem>
		{#if folder?.role === "junk"}
			<DropdownMenuItem onclick={() => mail.junk(false)}>{t("action.notJunk")}</DropdownMenuItem>
		{:else}
			<DropdownMenuItem onclick={() => mail.junk(true)}>{#snippet icon()}<OctagonAlert size={14} />{/snippet}{t("action.junk")}</DropdownMenuItem>
		{/if}
		{#if !many}
			<DropdownMenuItem onclick={() => saveMessage(ref.folder, ref.uid)}>{t("action.downloadEml")}</DropdownMenuItem>
		{/if}
		<DropdownMenuSeparator />
		<DropdownMenuItem variant="destructive" onclick={() => mail.remove()}>{#snippet icon()}<Trash2 size={14} />{/snippet}{t("action.delete")}</DropdownMenuItem>
		<DropdownMenuItem variant="destructive" onclick={() => mail.remove(undefined, true)}>{t("action.deleteForever")}</DropdownMenuItem>
	</DropdownMenu>
{/if}

<style>
	.lm-list {
		flex: 1;
		min-height: 0;
		border-right: 1px solid var(--ca-outline-subtle);
	}
	.lm-list__total {
		font-variant-numeric: tabular-nums;
	}
	.lm-list__tool {
		width: var(--lm-tool);
		height: var(--lm-tool);
		flex-shrink: 0;
		display: grid;
		place-items: center;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		cursor: pointer;
	}
	.lm-list__tool:hover {
		background: var(--lm-row-hover);
		color: var(--ca-text-primary);
	}
	.lm-list__tool--on {
		color: var(--ca-brand-text);
		box-shadow: inset 0 -2px 0 var(--lm-signal);
	}
	.lm-list__search {
		padding: var(--ca-space-2-5);
		border-bottom: 1px solid var(--ca-outline-subtle);
	}
	.lm-search {
		display: flex;
		align-items: center;
		gap: var(--ca-space-2);
		height: var(--lm-control);
		padding: 0 var(--ca-space-0-5) 0 var(--ca-space-3);
		border: 1px solid var(--ca-outline-default);
		background: var(--ca-surface-sunken);
		color: var(--ca-text-secondary);
	}
	.lm-search:focus-within {
		border-color: var(--ca-outline-strong);
		background: var(--ca-surface-base);
	}
	.lm-search input {
		flex: 1;
		min-width: 0;
		border: 0;
		outline: none;
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
	}
	.lm-search input::-webkit-search-cancel-button {
		display: none;
	}
	.lm-filters {
		display: flex;
		flex-direction: column;
		gap: var(--ca-space-3-5);
		width: var(--lm-filters-w);
		padding: var(--ca-space-1-5);
	}
	.lm-filters__row {
		display: flex;
		gap: var(--ca-space-2-5);
	}
	.lm-filters__field {
		display: flex;
		flex-direction: column;
		gap: var(--ca-space-1-5);
		flex: 1;
		min-width: 0;
		font-size: var(--ca-font-size-body2);
		color: var(--ca-text-secondary);
	}
	:global(.lm-input) {
		height: var(--lm-control);
		padding: 0 var(--ca-space-2-5);
		border: 1px solid var(--ca-outline-default);
		background: var(--ca-surface-base);
		color: var(--ca-text-primary);
		font: inherit;
		font-size: var(--ca-font-size-button);
		min-width: 0;
	}
	.lm-filters__actions {
		display: flex;
		justify-content: flex-end;
		gap: var(--ca-space-2-5);
	}
	.lm-list__scroll {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		position: relative;
		outline: none;
	}
	.lm-list__loading {
		display: grid;
		place-items: center;
		padding: var(--ca-space-16) 0;
	}
	.lm-list__space {
		position: relative;
	}
	.lm-row {
		position: absolute;
		left: 0;
		right: 0;
		display: flex;
		align-items: center;
		gap: var(--ca-space-2-5);
		padding: 0 var(--ca-space-3-5) 0 var(--ca-space-1-5);
		border-bottom: 1px solid var(--ca-outline-subtle);
		cursor: default;
		user-select: none;
		color: var(--ca-text-secondary);
		box-sizing: border-box;
	}
	.lm-row:hover {
		background: var(--lm-row-hover);
	}
	.lm-row--unread {
		color: var(--ca-text-primary);
		font-weight: 650;
	}
	.lm-row--unread::before {
		content: "";
		position: absolute;
		left: 0;
		top: 0;
		bottom: 0;
		width: var(--lm-accent);
		background: var(--lm-signal);
	}
	.lm-row--selected {
		background: var(--lm-row-selected);
	}
	.lm-row--open {
		background: var(--lm-row-open);
		color: var(--ca-text-primary);
	}
	.lm-row--focus {
		outline: 1px solid var(--ca-outline-strong);
		outline-offset: -1px;
	}
	.lm-row__check {
		width: var(--lm-check-w);
		flex-shrink: 0;
		display: grid;
		place-items: center;
		opacity: 0;
	}
	.lm-row:hover .lm-row__check,
	.lm-row__check--show,
	.lm-row--selected .lm-row__check {
		opacity: 1;
	}
	.lm-row__body {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: var(--ca-space-0-5);
	}
	.lm-row__line {
		display: flex;
		align-items: center;
		gap: var(--ca-space-2);
		min-width: 0;
	}
	.lm-row__from {
		flex: 1;
		color: inherit;
	}
	.lm-row__from--wide {
		flex: 0 0 26%;
		max-width: var(--lm-from-w);
	}
	.lm-row__subject {
		flex: 1;
	}
	.lm-row--compact .lm-row__subject {
		font-weight: inherit;
		color: inherit;
		font-size: var(--ca-font-size-button);
	}
	.lm-row__status {
		display: flex;
		align-items: center;
		gap: var(--ca-space-0-5);
		width: var(--lm-tool-sm);
		flex-shrink: 0;
	}
	.lm-row__date {
		flex-shrink: 0;
		font-size: var(--ca-font-size-body2);
		font-variant-numeric: tabular-nums;
		color: var(--ca-text-secondary);
		font-weight: 500;
	}
	.lm-row--unread .lm-row__date {
		color: var(--ca-text-primary);
		font-weight: 650;
	}
	.lm-row__size {
		width: var(--lm-size-w);
		text-align: right;
		flex-shrink: 0;
		font-weight: 400;
	}
	.lm-row__folder {
		flex-shrink: 0;
		font-size: var(--ca-font-size-caption);
		font-weight: 500;
		padding: 0 var(--ca-space-1-5);
		border: 1px solid var(--ca-outline-default);
		color: var(--ca-text-secondary);
		max-width: var(--lm-folder-tag-w);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.lm-row__prio {
		color: var(--lm-danger);
		font-weight: 800;
	}
	:global(.lm-row__icon) {
		flex-shrink: 0;
		color: var(--ca-text-secondary);
	}
	.lm-row__flag {
		width: var(--lm-tool-sm);
		height: var(--lm-tool-sm);
		flex-shrink: 0;
		display: grid;
		place-items: center;
		padding: 0;
		border: 0;
		background: transparent;
		color: var(--ca-text-tertiary, var(--ca-text-secondary));
		cursor: pointer;
		opacity: 0;
	}
	.lm-row:hover .lm-row__flag,
	.lm-row__flag--on {
		opacity: 1;
	}
	.lm-row__flag--on {
		color: var(--lm-flag);
	}
	.lm-row__flag--on :global(svg) {
		fill: currentColor;
	}
	.lm-row__placeholder span {
		display: block;
		height: var(--ca-space-3);
		background: var(--ca-surface-sunken);
	}
	.lm-row__placeholder span:first-child {
		width: 40%;
	}
	.lm-row__placeholder span:last-child {
		width: 75%;
	}
	:global(.lm-drag-ghost) {
		position: fixed;
		top: -100px;
		padding: var(--ca-space-2) var(--ca-space-3-5);
		background: var(--ca-brand-fill);
		color: var(--ca-brand-contrast);
		font-weight: 650;
		font-size: var(--ca-font-size-button);
	}
</style>
