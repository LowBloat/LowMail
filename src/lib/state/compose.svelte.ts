/**
 * The message being written: opening (new, reply, forward, draft), the
 * signature of the chosen identity, attachments, autosave and sending.
 */
import { toast } from "@codeatlasdev/ui";
import { call, upload as uploadFiles } from "../api";
import { errorText, i18n, t } from "../i18n/index.svelte";
import { escapeHtml, htmlToText } from "../format";
import type { Address, ComposeMode, Draft, DraftAttachment, Identity, MessageRef, SendResult } from "../types";
import { app } from "./app.svelte";
import { choose, confirm } from "./dialogs.svelte";
import { mail } from "./mail.svelte";

const SIGNATURE_CLASS = "lm-signature";
const TEXT_SIGNATURE = "\n-- \n";

export interface Preset {
	to?: Address[];
	cc?: Address[];
	bcc?: Address[];
	subject?: string;
	body?: string;
}

function emptyDraft(): Draft {
	return {
		mode: "new",
		identityId: null,
		from: null,
		to: [],
		cc: [],
		bcc: [],
		replyTo: [],
		subject: "",
		html: "",
		text: "",
		attachments: [],
		source: null,
		inReplyTo: null,
		references: [],
		priority: "normal",
		requestReceipt: false,
		draftRef: null,
		messageId: null,
	};
}

function signatureHtml(identity: Identity | undefined): string {
	const sig = identity?.signature?.trim();
	if (!sig) return "";
	return `<div class="${SIGNATURE_CLASS}">-- <br>${sig}</div>`;
}

function signatureText(identity: Identity | undefined): string {
	const sig = identity?.signature?.trim();
	return sig ? `${TEXT_SIGNATURE}${htmlToText(sig)}` : "";
}

export interface PendingUpload {
	id: number;
	name: string;
	size: number;
}

class ComposeState {
	draft = $state<Draft | null>(null);
	/** Shown (false: kept aside while the user reads other mail). */
	visible = $state(false);
	loading = $state(false);
	saving = $state(false);
	sending = $state(false);
	savedAt = $state<number | null>(null);
	uploading = $state<PendingUpload[]>([]);
	showCc = $state(false);
	showBcc = $state(false);
	/** Bumped when the body is replaced from outside, so the editor reloads it. */
	bodyVersion = $state(0);
	private savedSnapshot = "";
	private autosaveTimer: ReturnType<typeof setInterval> | undefined;
	private uploadSeq = 0;

	get open() {
		return this.draft !== null;
	}

	get html() {
		return this.draft?.html !== null && this.draft?.html !== undefined;
	}

	get identity(): Identity | undefined {
		const d = this.draft;
		return app.identities.find((i) => i.id === d?.identityId) ?? app.defaultIdentity;
	}

	private snapshot() {
		return JSON.stringify(this.draft);
	}

	get dirty() {
		return this.draft !== null && this.snapshot() !== this.savedSnapshot;
	}

	async open(mode: ComposeMode = "new", source: MessageRef | null = null, preset: Preset = {}) {
		if (this.draft && this.dirty && !(await this.closeAsking())) return;
		this.loading = true;
		this.visible = true;
		app.view = "mail";
		try {
			const style = {
				locale: i18n.lang,
				tzOffsetMinutes: -new Date().getTimezoneOffset(),
				replyOnTop: app.settings.replyOnTop,
			};
			const d = await call<Draft>("compose.prepare", { mode, source, html: app.settings.composeHtml, style });
			const draft = { ...emptyDraft(), ...d };
			if (preset.to) draft.to = preset.to;
			if (preset.cc) draft.cc = preset.cc;
			if (preset.bcc) draft.bcc = preset.bcc;
			if (preset.subject) draft.subject = preset.subject;
			if (mode !== "draft" && mode !== "editAsNew") this.insertSignature(draft, preset.body);
			else if (preset.body) draft.text = preset.body;
			this.draft = draft;
			this.showCc = draft.cc.length > 0;
			this.showBcc = draft.bcc.length > 0;
			this.savedSnapshot = mode === "draft" ? this.snapshot() : "";
			this.savedAt = null;
			this.bodyVersion++;
			this.startAutosave();
		} catch (e) {
			toast.error(errorText(e));
			this.visible = false;
		} finally {
			this.loading = false;
		}
	}

	private insertSignature(draft: Draft, body?: string) {
		const identity = app.identities.find((i) => i.id === draft.identityId) ?? app.defaultIdentity;
		if (draft.html !== null) {
			const sig = signatureHtml(identity);
			const intro = body ? `<p>${escapeHtml(body).replace(/\n/g, "<br>")}</p>` : "<p><br></p>";
			const rest = (draft.html ?? "").replace(/^(<p><br><\/p>)+/, "");
			const quoted = rest.trim().length > 0;
			if (quoted && app.settings.replyOnTop) draft.html = `${intro}${sig ? `<p><br></p>${sig}` : ""}<p><br></p>${rest}`;
			else if (quoted) draft.html = `${rest}${intro}${sig}`;
			else draft.html = `${intro}${sig ? `<p><br></p>${sig}` : ""}`;
		} else {
			const sig = signatureText(identity);
			const intro = body ?? "";
			const rest = draft.text.replace(/^\n+/, "");
			if (rest && app.settings.replyOnTop) draft.text = `${intro}\n${sig}\n\n${rest}`;
			else if (rest) draft.text = `${rest}\n${intro}${sig}`;
			else draft.text = `${intro}\n${sig}`;
		}
	}

	/** Switches the sender, swapping the signature in the body. */
	setIdentity(id: string) {
		const d = this.draft;
		if (!d) return;
		const old = this.identity;
		d.identityId = id;
		const next = this.identity;
		d.from = next ? { name: next.name, email: next.email } : d.from;
		if (d.html !== null) {
			const doc = new DOMParser().parseFromString(`<body>${d.html}</body>`, "text/html");
			const block = doc.querySelector(`.${SIGNATURE_CLASS}`);
			const sig = signatureHtml(next);
			if (block) {
				if (sig) block.outerHTML = sig;
				else block.remove();
			} else if (sig) {
				doc.body.insertAdjacentHTML(app.settings.replyOnTop ? "afterbegin" : "beforeend", `<p><br></p>${sig}`);
			}
			d.html = doc.body.innerHTML;
		} else {
			const oldSig = signatureText(old);
			const sig = signatureText(next);
			d.text = oldSig && d.text.includes(oldSig) ? d.text.replace(oldSig, sig) : d.text + sig;
		}
		this.bodyVersion++;
	}

	/** Switches between HTML and plain text, converting the body. */
	setHtml(on: boolean) {
		const d = this.draft;
		if (!d || on === (d.html !== null)) return;
		if (on) {
			d.html = escapeHtml(d.text).replace(/\n/g, "<br>");
			d.text = "";
		} else {
			d.text = htmlToText(d.html ?? "");
			d.html = null;
		}
		this.bodyVersion++;
	}

	async addFiles(files: File[]) {
		if (!this.draft || !files.length) return;
		const entries = files.map((f) => ({ id: ++this.uploadSeq, name: f.name, size: f.size }));
		this.uploading = [...this.uploading, ...entries];
		try {
			const uploads = await uploadFiles(files);
			for (const u of uploads) this.attach(u);
		} catch (e) {
			toast.error(errorText(e));
		} finally {
			const ids = new Set(entries.map((e) => e.id));
			this.uploading = this.uploading.filter((u) => !ids.has(u.id));
		}
	}

	attach(u: { id: string; filename: string; contentType: string; size: number }) {
		this.draft?.attachments.push({ source: "upload", id: u.id, filename: u.filename, contentType: u.contentType, size: u.size });
	}

	removeAttachment(a: DraftAttachment) {
		const d = this.draft;
		if (!d) return;
		d.attachments = d.attachments.filter((x) => x !== a);
	}

	private startAutosave() {
		clearInterval(this.autosaveTimer);
		const secs = app.settings.autosaveSeconds;
		if (secs > 0) {
			this.autosaveTimer = setInterval(() => {
				if (this.dirty && !this.saving && !this.sending && this.hasContent()) void this.save(true);
			}, secs * 1000);
		}
	}

	private hasContent() {
		const d = this.draft;
		if (!d) return false;
		const body = d.html !== null ? htmlToText(d.html) : d.text;
		return !!(d.subject.trim() || d.to.length || d.attachments.length || body.replace(/--\s*$/m, "").trim().length > 2);
	}

	async save(quiet = false) {
		const d = this.draft;
		if (!d || this.saving) return;
		this.saving = true;
		const snapshot = this.snapshot();
		try {
			const saved = await call<Draft>("compose.saveDraft", { draft: $state.snapshot(d) });
			if (this.draft === d) {
				d.draftRef = saved.draftRef;
				d.messageId = saved.messageId;
				// Edits made while saving stay unsaved.
				this.savedSnapshot = snapshot === this.snapshot() ? this.snapshot() : JSON.stringify({ ...JSON.parse(snapshot), draftRef: saved.draftRef, messageId: saved.messageId });
				this.savedAt = Date.now();
			}
			if (!quiet) toast.success(t("compose.saved"));
			const drafts = mail.folderByRole("drafts");
			if (drafts) void mail.refreshCounts([drafts.path]);
		} catch (e) {
			if (!quiet) toast.error(errorText(e));
		} finally {
			this.saving = false;
		}
	}

	private mentionsAttachment(d: Draft) {
		const body = d.html !== null ? htmlToText(d.html.split("<blockquote")[0]) : d.text.split(/\n>/)[0];
		return /\b(anexo|anexa|anexad[oa]s?|em anexo|segue anexo|attach(ed|ment|ments)?|enclosed)\b/i.test(`${d.subject}\n${body}`);
	}

	async send() {
		const d = this.draft;
		if (!d || this.sending) return;
		if (!d.to.length && !d.cc.length && !d.bcc.length) {
			toast.error(t("compose.noRecipients"));
			return;
		}
		if (!d.subject.trim() && !(await confirm({ title: t("compose.noSubject.title"), description: t("compose.noSubject.body"), confirmLabel: t("compose.sendAnyway") }))) return;
		if (!d.attachments.length && this.mentionsAttachment(d) && !(await confirm({ title: t("compose.noAttachment.title"), description: t("compose.noAttachment.body"), confirmLabel: t("compose.sendAnyway") }))) return;
		if (this.uploading.length) {
			toast.info(t("compose.waitUploads"));
			return;
		}
		this.sending = true;
		try {
			const res = await call<SendResult>("compose.send", { draft: $state.snapshot(d), saveCopy: app.settings.saveSent });
			for (const w of res.warnings ?? []) toast.warning(w, { duration: 10_000 });
			toast.success(t("compose.sent"));
			this.reset();
			void mail.refreshCounts();
			if (mail.current?.role === "sent" || mail.current?.role === "drafts") mail.refresh();
			void app.loadIdentities();
		} catch (e) {
			toast.error(errorText(e), { duration: 10_000 });
		} finally {
			this.sending = false;
		}
	}

	/** Closes the editor; asks to keep the draft when it has unsaved changes. */
	async closeAsking(): Promise<boolean> {
		const d = this.draft;
		if (!d) return true;
		if (this.dirty && this.hasContent()) {
			const answer = await choose({
				title: t("compose.keep.title"),
				description: t("compose.keep.body"),
				choices: [
					{ value: "discard", label: t("compose.keep.discard"), variant: "ghost" },
					{ value: "edit", label: t("compose.keep.edit"), variant: "outlined" },
					{ value: "save", label: t("compose.keep.save"), variant: "filled" },
				],
			});
			if (answer === null || answer === "edit") {
				this.show();
				return false;
			}
			if (answer === "save") await this.save(true);
			else await this.discard(false);
		}
		this.reset();
		return true;
	}

	async discard(ask = true) {
		const d = this.draft;
		if (!d) return;
		if (ask && this.hasContent() && !(await confirm({ title: t("compose.discard.title"), description: t("compose.discard.body"), confirmLabel: t("compose.discard.confirm"), destructive: true }))) return;
		if (d.draftRef) {
			try {
				await call("compose.discardDraft", { folder: d.draftRef.folder, uid: d.draftRef.uid });
				mail.refresh();
			} catch {
				// The draft stays in Drafts; nothing is lost.
			}
		}
		for (const a of d.attachments) if (a.source === "upload") void call("uploads.delete", { id: a.id }).catch(() => {});
		this.reset();
	}

	private reset() {
		clearInterval(this.autosaveTimer);
		this.draft = null;
		this.visible = false;
		this.savedSnapshot = "";
		this.savedAt = null;
		this.uploading = [];
	}

	hide() {
		this.visible = false;
	}

	show() {
		if (this.draft) {
			this.visible = true;
			app.view = "mail";
		}
	}

	/** Opens the editor for a `mailto:` link. */
	async mailto(href: string) {
		const url = new URL(href);
		const params = url.searchParams;
		const list = (v: string | null) =>
			(v ?? "")
				.split(",")
				.map((s) => decodeURIComponent(s).trim())
				.filter(Boolean)
				.map((email) => ({ email, name: null }));
		await this.open("new", null, {
			to: list(decodeURIComponent(url.pathname) || null),
			cc: list(params.get("cc")),
			bcc: list(params.get("bcc")),
			subject: params.get("subject") ?? undefined,
			body: params.get("body") ?? undefined,
		});
	}
}

export const compose = new ComposeState();
