/**
 * Folders, the message list and the open message. Rows are keyed by
 * `folder\u0000uid` so a search across folders works like any folder.
 */
import { toast } from "@codeatlasdev/ui";
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { call, listen } from "../api";
import { errorText, t } from "../i18n/index.svelte";
import { displayName } from "../format";
import { notify } from "../notify";
import type { Filter, Folder, FolderCount, MailEvent, Message, MessagePage, MessageSummary, Sort } from "../types";
import { app } from "./app.svelte";
import { confirm } from "./dialogs.svelte";

export type RowKey = string;
export const keyOf = (folder: string, uid: number): RowKey => `${folder}\u0000${uid}`;
export function parseKey(key: RowKey): { folder: string; uid: number } {
	const i = key.lastIndexOf("\u0000");
	return { folder: key.slice(0, i), uid: Number(key.slice(i + 1)) };
}

export function emptyFilter(): Filter {
	return { text: "", scope: "headers", unread: false, flagged: false, answered: false, unanswered: false, since: null, before: null, larger: null };
}

export function filterActive(f: Filter) {
	return !!f.text.trim() || f.unread || f.flagged || f.answered || f.unanswered || !!f.since || !!f.before || !!f.larger;
}

/** UIDs of `keys` grouped by folder. */
function byFolder(keys: Iterable<RowKey>): Map<string, number[]> {
	const out = new Map<string, number[]>();
	for (const k of keys) {
		const { folder, uid } = parseKey(k);
		if (!out.has(folder)) out.set(folder, []);
		out.get(folder)!.push(uid);
	}
	return out;
}

const ROW_BATCH = 100;

/** A deeply reactive copy, so flag changes on a row update the list. */
function reactive<T>(value: T): T {
	const proxy = $state(value);
	return proxy;
}

class MailState {
	folders = $state<Folder[]>([]);
	foldersLoading = $state(false);
	/** Folder shown in the list; "" while searching every folder. */
	folder = $state("INBOX");
	searchEverywhere = $state(false);
	filter = $state<Filter>(emptyFilter());
	sort = $state<Sort>({ field: "date", descending: true });

	keys = $state<RowKey[]>([]);
	rows = new SvelteMap<RowKey, MessageSummary>();
	total = $state(0);
	listLoading = $state(false);
	listError = $state<string | null>(null);
	private listToken = 0;
	private pending = new Set<RowKey>();

	selected = new SvelteSet<RowKey>();
	anchor: RowKey | null = null;
	focused = $state<RowKey | null>(null);

	open = $state<RowKey | null>(null);
	message = $state<Message | null>(null);
	messageLoading = $state(false);
	messageError = $state<string | null>(null);
	/** Remote images allowed for this message by a click. */
	allowRemote = $state(false);
	private messageToken = 0;
	private markTimer: ReturnType<typeof setTimeout> | undefined;
	private stopEvents: (() => void) | null = null;
	private refreshTimer: ReturnType<typeof setTimeout> | undefined;

	get current(): Folder | undefined {
		return this.folders.find((f) => f.path === this.folder);
	}

	folderByRole(role: Folder["role"]) {
		return this.folders.find((f) => f.role === role);
	}

	get inboxUnread() {
		return this.folders.find((f) => f.role === "inbox")?.unread ?? 0;
	}

	/** Row keys in list order that are selected, or the focused row. */
	get targets(): RowKey[] {
		if (this.selected.size) return this.keys.filter((k) => this.selected.has(k));
		return this.focused ? [this.focused] : this.open ? [this.open] : [];
	}

	// ---- start ----------------------------------------------------------

	async start() {
		this.stopEvents?.();
		this.stopEvents = null;
		this.reset();
		this.sort = { ...app.settings.sort };
		await this.loadFolders();
		await this.openFolder("INBOX");
		try {
			this.stopEvents = await listen(
				(e) => this.onEvent(e),
				() => (app.online = false),
			);
		} catch {
			// Without events the list still refreshes on demand.
		}
	}

	stop() {
		this.stopEvents?.();
		this.stopEvents = null;
	}

	private reset() {
		// Answers still on their way belong to the previous account.
		this.listToken++;
		this.closeMessage();
		this.folders = [];
		this.keys = [];
		this.rows.clear();
		this.selected.clear();
		this.open = null;
		this.message = null;
		this.focused = null;
		this.filter = emptyFilter();
		this.searchEverywhere = false;
	}

	// ---- folders --------------------------------------------------------

	async loadFolders() {
		this.foldersLoading = true;
		try {
			this.folders = await call<Folder[]>("folders.list", { counts: true });
		} catch (e) {
			toast.error(errorText(e));
		} finally {
			this.foldersLoading = false;
		}
		this.updateTitle();
	}

	applyCounts(counts: FolderCount[]) {
		for (const c of counts) {
			const f = this.folders.find((x) => x.path === c.path);
			if (f) {
				f.total = c.total;
				f.unread = c.unread;
			}
		}
		this.updateTitle();
	}

	private adjustUnread(folder: string, delta: number) {
		const f = this.folders.find((x) => x.path === folder);
		if (f) f.unread = Math.max(0, f.unread + delta);
		this.updateTitle();
	}

	updateTitle() {
		const n = this.inboxUnread;
		document.title = n > 0 ? `(${n}) ${app.productName}` : app.productName;
	}

	async refreshCounts(paths?: string[]) {
		const list = paths ?? this.folders.filter((f) => f.selectable).map((f) => f.path);
		if (!list.length) return;
		try {
			this.applyCounts(await call<FolderCount[]>("folders.counts", { paths: list }));
		} catch {
			// Counters refresh again on the next change.
		}
	}

	// ---- list -----------------------------------------------------------

	async openFolder(path: string, keepFilter = false) {
		this.folder = path;
		this.searchEverywhere = false;
		if (!keepFilter) this.filter = emptyFilter();
		this.selected.clear();
		this.anchor = null;
		this.focused = null;
		this.closeMessage();
		app.view = "mail";
		await this.loadList();
	}

	async loadList(keepPosition = false) {
		const token = ++this.listToken;
		this.listLoading = true;
		this.listError = null;
		try {
			if (this.searchEverywhere) {
				const items = await call<MessageSummary[]>("messages.searchAll", { filter: $state.snapshot(this.filter) });
				if (token !== this.listToken) return;
				this.rows.clear();
				for (const m of items) this.rows.set(keyOf(m.folder, m.uid), reactive(m));
				this.keys = items.map((m) => keyOf(m.folder, m.uid));
				this.total = items.length;
			} else {
				const folder = this.folder;
				const page = await call<MessagePage>("messages.list", {
					folder,
					offset: 0,
					limit: app.settings.pageSize || ROW_BATCH,
					sort: $state.snapshot(this.sort),
					filter: $state.snapshot(this.filter),
				});
				if (token !== this.listToken) return;
				const keep = keepPosition ? new Map(this.rows) : null;
				this.rows.clear();
				for (const m of page.items) this.rows.set(keyOf(folder, m.uid), reactive(m));
				this.keys = page.uids.map((u) => keyOf(folder, u));
				// Rows already known keep showing while the visible range reloads.
				if (keep) for (const k of this.keys) if (!this.rows.has(k) && keep.has(k)) this.rows.set(k, keep.get(k)!);
				this.total = page.total;
				const f = this.folders.find((x) => x.path === folder);
				if (f && !filterActive(this.filter)) f.total = page.total;
			}
			const present = new Set(this.keys);
			for (const k of [...this.selected]) if (!present.has(k)) this.selected.delete(k);
			if (this.focused && !present.has(this.focused)) this.focused = null;
			app.online = true;
		} catch (e) {
			if (token === this.listToken) this.listError = errorText(e);
		} finally {
			if (token === this.listToken) this.listLoading = false;
		}
	}

	/** Loads the summaries of rows `from..to` that are not loaded yet. */
	async ensureRows(from: number, to: number) {
		const missing = this.keys.slice(Math.max(0, from), to).filter((k) => !this.rows.has(k) && !this.pending.has(k));
		if (!missing.length) return;
		// Fetch a whole batch around the gap, so scrolling stays smooth.
		const start = this.keys.indexOf(missing[0]);
		const batch = this.keys.slice(start, start + ROW_BATCH).filter((k) => !this.rows.has(k) && !this.pending.has(k));
		batch.forEach((k) => this.pending.add(k));
		const token = this.listToken;
		try {
			for (const [folder, uids] of byFolder(batch)) {
				const items = await call<MessageSummary[]>("messages.summaries", { folder, uids });
				if (token !== this.listToken) return;
				for (const m of items) this.rows.set(keyOf(folder, m.uid), reactive(m));
			}
		} catch {
			// Rows stay as placeholders; scrolling again retries.
		} finally {
			batch.forEach((k) => this.pending.delete(k));
		}
	}

	search(filter: Filter, everywhere = false) {
		this.filter = filter;
		this.searchEverywhere = everywhere;
		this.selected.clear();
		this.closeMessage();
		void this.loadList();
	}

	clearSearch() {
		this.filter = emptyFilter();
		this.searchEverywhere = false;
		if (!this.folder) this.folder = "INBOX";
		void this.loadList();
	}

	setSort(sort: Sort) {
		this.sort = sort;
		void app.saveSettings({ sort });
		void this.loadList();
	}

	refresh() {
		void this.loadList(true);
		void this.refreshCounts();
	}

	// ---- selection ------------------------------------------------------

	click(key: RowKey, e: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) {
		if (e.shiftKey && this.anchor) {
			const a = this.keys.indexOf(this.anchor);
			const b = this.keys.indexOf(key);
			if (a >= 0 && b >= 0) {
				if (!(e.ctrlKey || e.metaKey)) this.selected.clear();
				for (let i = Math.min(a, b); i <= Math.max(a, b); i++) this.selected.add(this.keys[i]);
				this.focused = key;
				return;
			}
		}
		if (e.ctrlKey || e.metaKey) {
			if (this.selected.size === 0 && this.open && this.open !== key) this.selected.add(this.open);
			if (this.selected.has(key)) this.selected.delete(key);
			else this.selected.add(key);
			this.anchor = key;
			this.focused = key;
			return;
		}
		this.selected.clear();
		this.anchor = key;
		this.focused = key;
		void this.openMessage(key);
	}

	toggle(key: RowKey) {
		if (this.selected.has(key)) this.selected.delete(key);
		else this.selected.add(key);
		this.anchor = key;
	}

	selectAll() {
		for (const k of this.keys) this.selected.add(k);
	}

	selectNone() {
		this.selected.clear();
	}

	/** Moves the cursor (keyboard j/k or arrows). */
	move(step: number, extend = false) {
		if (!this.keys.length) return;
		const current = this.focused ?? this.open;
		let i = current ? this.keys.indexOf(current) : -1;
		i = Math.min(this.keys.length - 1, Math.max(0, i + step));
		const key = this.keys[i];
		if (extend) {
			this.selected.add(current ?? key);
			this.selected.add(key);
		} else {
			this.selected.clear();
		}
		this.focused = key;
		this.anchor = key;
		if (!extend && app.settings.layout !== "list") void this.openMessage(key);
	}

	// ---- reading --------------------------------------------------------

	closeMessage() {
		clearTimeout(this.markTimer);
		this.messageToken++;
		this.open = null;
		this.message = null;
		this.messageError = null;
		this.messageLoading = false;
		this.allowRemote = false;
	}

	private trustsSender(m: { from: { email: string }[] }) {
		const mode = app.settings.showImages;
		if (mode === "always") return true;
		if (mode === "never") return false;
		const from = m.from[0]?.email.toLowerCase();
		return !!from && (app.settings.trustedSenders.includes(from) || app.isMine(from));
	}

	async openMessage(key: RowKey, allowRemote?: boolean) {
		clearTimeout(this.markTimer);
		const token = ++this.messageToken;
		const { folder, uid } = parseKey(key);
		const summary = this.rows.get(key);
		// Drafts open in the editor.
		const draftsFolder = this.folderByRole("drafts")?.path;
		if (folder === draftsFolder || summary?.flags.draft) {
			const { compose } = await import("./compose.svelte");
			await compose.open("draft", { folder, uid });
			return;
		}
		this.open = key;
		this.focused = key;
		this.messageError = null;
		this.messageLoading = true;
		if (this.message && keyOf(this.message.folder, this.message.uid) !== key) this.message = null;
		const remote = allowRemote ?? (summary ? this.trustsSender(summary) : false);
		this.allowRemote = remote;
		const delay = app.settings.markReadDelay;
		try {
			const m = await call<Message>("messages.get", { folder, uid, allowRemote: remote, markSeen: delay === 0 });
			if (token !== this.messageToken) return;
			if (allowRemote === undefined && !remote && this.trustsSender(m) && m.remoteContent) {
				// The sender is trusted but the summary was not loaded: ask again with images.
				// The second load does the rest (read flag, receipt), once.
				if (summary && !summary.flags.seen && delay === 0) {
					summary.flags.seen = true;
					this.adjustUnread(folder, -1);
				}
				void this.openMessage(key, true);
				return;
			}
			this.message = m;
			const wasUnread = summary && !summary.flags.seen;
			if (delay === 0 && wasUnread) {
				summary.flags.seen = true;
				this.adjustUnread(folder, -1);
			} else if (delay > 0 && !m.flags.seen) {
				this.markTimer = setTimeout(() => {
					if (this.open === key) void this.setFlags([key], { seen: true });
				}, delay * 1000);
			}
			if (m.receiptTo && !m.flags.keywords?.some((k) => k.toLowerCase() === "$mdnsent") && app.settings.readReceipts === "always") {
				void this.sendReceipt();
			}
		} catch (e) {
			if (token === this.messageToken) this.messageError = errorText(e);
		} finally {
			if (token === this.messageToken) this.messageLoading = false;
		}
	}

	showImages(always = false) {
		if (!this.open || !this.message) return;
		if (always) {
			const from = this.message.from[0]?.email.toLowerCase();
			if (from && !app.settings.trustedSenders.includes(from)) {
				void app.saveSettings({ trustedSenders: [...app.settings.trustedSenders, from] });
			}
		}
		void this.openMessage(this.open, true);
	}

	async sendReceipt() {
		const m = this.message;
		if (!m) return;
		try {
			await call("messages.sendReceipt", { folder: m.folder, uid: m.uid });
			m.flags.keywords = [...(m.flags.keywords ?? []), "$MDNSent"];
			toast.success(t("message.receiptSent"));
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	// ---- actions --------------------------------------------------------

	async setFlags(keys: RowKey[], change: { seen?: boolean; flagged?: boolean }) {
		if (!keys.length) return;
		const before = new Map(keys.map((k) => [k, this.rows.get(k)?.flags ? { ...this.rows.get(k)!.flags } : null]));
		for (const k of keys) {
			const row = this.rows.get(k);
			if (!row) continue;
			if (change.seen !== undefined && row.flags.seen !== change.seen) {
				row.flags.seen = change.seen;
				this.adjustUnread(row.folder, change.seen ? -1 : 1);
			}
			if (change.flagged !== undefined) row.flags.flagged = change.flagged;
		}
		if (this.message && keys.includes(keyOf(this.message.folder, this.message.uid))) {
			if (change.seen !== undefined) this.message.flags.seen = change.seen;
			if (change.flagged !== undefined) this.message.flags.flagged = change.flagged;
		}
		try {
			for (const [folder, uids] of byFolder(keys)) await call("messages.flags", { folder, uids, change });
		} catch (e) {
			toast.error(errorText(e));
			for (const [k, flags] of before) {
				const row = this.rows.get(k);
				if (row && flags) row.flags = flags;
			}
			void this.refreshCounts();
		}
	}

	toggleRead(keys = this.targets) {
		const anyUnread = keys.some((k) => this.rows.get(k)?.flags.seen === false);
		return this.setFlags(keys, { seen: anyUnread });
	}

	toggleFlag(keys = this.targets) {
		const anyUnflagged = keys.some((k) => !this.rows.get(k)?.flags.flagged);
		return this.setFlags(keys, { flagged: anyUnflagged });
	}

	/** Removes rows from the list and opens the next one, as mail apps do. */
	private removeRows(keys: RowKey[]) {
		const gone = new Set(keys);
		const openGone = this.open !== null && gone.has(this.open);
		// The next row: the first one kept at or after the cursor, else the
		// last one kept before it.
		const before = this.keys;
		const cursor = this.focused ?? this.open;
		const at = cursor ? before.indexOf(cursor) : -1;
		let next: RowKey | null = null;
		if (at >= 0) {
			next = before.slice(at).find((k) => !gone.has(k)) ?? [...before.slice(0, at)].reverse().find((k) => !gone.has(k)) ?? null;
		}
		for (const k of keys) {
			const row = this.rows.get(k);
			if (row && !row.flags.seen) this.adjustUnread(row.folder, -1);
			const f = row && this.folders.find((x) => x.path === row.folder);
			if (f) f.total = Math.max(0, f.total - 1);
		}
		this.keys = this.keys.filter((k) => !gone.has(k));
		this.total = Math.max(0, this.total - keys.length);
		for (const k of keys) {
			this.selected.delete(k);
			this.rows.delete(k);
		}
		next ??= this.keys[0] ?? null;
		this.focused = next;
		if (openGone) {
			if (next && app.settings.layout !== "list") void this.openMessage(next);
			else this.closeMessage();
		}
	}

	/** Runs a move-like action, then offers to undo it when the server said
	 * where the messages went (COPYUID). */
	private async run(keys: RowKey[], action: (folder: string, uids: number[]) => Promise<unknown>, done?: string) {
		if (!keys.length) return;
		this.removeRows(keys);
		const moves: { from: string; to: string; uids: number[] }[] = [];
		try {
			for (const [folder, uids] of byFolder(keys)) {
				const moved = (await action(folder, uids)) as { folder?: string; uids?: number[] } | null;
				if (moved?.folder && moved.uids?.length) moves.push({ from: folder, to: moved.folder, uids: moved.uids });
			}
			if (done) {
				const undo = moves.length ? { label: t("action.undo"), onClick: () => void this.undo(moves) } : undefined;
				toast.success(done, { action: undo, duration: undo ? 8000 : 4000 });
			}
		} catch (e) {
			toast.error(errorText(e));
			void this.loadList(true);
		}
		void this.refreshCounts();
	}

	private async undo(moves: { from: string; to: string; uids: number[] }[]) {
		try {
			for (const m of moves) await call("messages.move", { folder: m.to, uids: m.uids, dest: m.from });
			toast.success(t("toast.undone"));
		} catch (e) {
			toast.error(errorText(e));
		}
		void this.loadList(true);
		void this.refreshCounts();
	}

	async remove(keys = this.targets, permanent = false) {
		const n = keys.length;
		if (!n) return;
		const inTrash = keys.every((k) => parseKey(k).folder === this.folderByRole("trash")?.path);
		const forGood = permanent || inTrash || !this.folderByRole("trash");
		if (forGood && app.settings.confirmPermanentDelete) {
			const ok = await confirm({
				title: t("confirm.deleteForever.title", { n }),
				description: t("confirm.deleteForever.body"),
				confirmLabel: t("action.deleteForever"),
				destructive: true,
			});
			if (!ok) return;
		}
		await this.run(keys, (folder, uids) => call("messages.delete", { folder, uids, permanent: forGood }), forGood ? t("toast.deleted", { n }) : t("toast.trashed", { n }));
	}

	async moveTo(dest: string, keys = this.targets) {
		const moving = keys.filter((k) => parseKey(k).folder !== dest);
		const target = this.folders.find((f) => f.path === dest);
		await this.run(moving, (folder, uids) => call("messages.move", { folder, uids, dest }), t("toast.moved", { n: moving.length, folder: target?.name ?? dest }));
	}

	async copyTo(dest: string, keys = this.targets) {
		try {
			for (const [folder, uids] of byFolder(keys)) await call("messages.copy", { folder, uids, dest });
			toast.success(t("toast.copied", { n: keys.length }));
			void this.refreshCounts([dest]);
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	async archive(keys = this.targets) {
		const archive = this.folderByRole("archive")?.path;
		await this.run(keys.filter((k) => parseKey(k).folder !== archive), (folder, uids) => call("messages.archive", { folder, uids }), t("toast.archived", { n: keys.length }));
		if (!archive) void this.loadFolders();
	}

	async junk(isJunk: boolean, keys = this.targets) {
		await this.run(keys, (folder, uids) => call("messages.junk", { folder, uids, junk: isJunk }), isJunk ? t("toast.junk", { n: keys.length }) : t("toast.notJunk", { n: keys.length }));
		if (!this.folderByRole("junk")) void this.loadFolders();
	}

	// ---- events ---------------------------------------------------------

	private onEvent(e: MailEvent) {
		switch (e.type) {
			case "newMail": {
				this.applyCounts([{ path: e.folder, total: (this.folders.find((f) => f.path === e.folder)?.total ?? 0) + e.latest.length, unread: e.unread }]);
				if (this.folder === e.folder && !this.searchEverywhere) this.scheduleRefresh();
				if (app.settings.notifications && e.latest.length) {
					const first = e.latest[0];
					const title = e.latest.length === 1 ? displayName(first.from[0]) || t("notify.newMail") : t("notify.newMessages", { n: e.latest.length });
					const body = e.latest.length === 1 ? first.subject || t("message.noSubject") : e.latest.map((m) => `${displayName(m.from[0])}: ${m.subject}`).slice(0, 3).join("\n");
					void notify(title, body, () => {
						void this.openFolder(e.folder).then(() => this.openMessage(keyOf(e.folder, first.uid)));
					});
				}
				break;
			}
			case "folderChanged":
				if (this.folder === e.folder && !this.searchEverywhere) this.scheduleRefresh();
				void this.refreshCounts([e.folder]);
				break;
			case "counts":
				this.applyCounts(e.folders);
				break;
			case "connection":
				app.online = e.online;
				if (e.online) this.refresh();
				break;
		}
	}

	private scheduleRefresh() {
		clearTimeout(this.refreshTimer);
		this.refreshTimer = setTimeout(() => void this.loadList(true), 400);
	}
}

export const mail = new MailState();

app.onAccountOpen(() => mail.start());
