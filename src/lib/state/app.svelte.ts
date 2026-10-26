/**
 * Who is signed in, which account is shown, which screen is open and the
 * account's preferences.
 */
import { colorScheme, toast } from "@codeatlasdev/ui";
import { ApiError, call, desktop, isDesktop, setAccount, setSessionExpiredHandler, web, type WebConfig } from "../api";
import { errorText, i18n, t } from "../i18n/index.svelte";
import type { AccountConfig, AccountInfo, Identity, Settings } from "../types";

export type View = "mail" | "contacts" | "settings";

export const DEFAULT_SETTINGS: Settings = {
	language: "auto",
	theme: "system",
	layout: "columns",
	density: "compact",
	markReadDelay: 0,
	showImages: "contacts",
	trustedSenders: [],
	composeHtml: true,
	replyOnTop: true,
	saveSent: true,
	autosaveSeconds: 60,
	confirmPermanentDelete: true,
	readReceipts: "ask",
	notifications: true,
	pageSize: 100,
	sort: { field: "date", descending: true },
};

const LAST_ACCOUNT_KEY = "lowmail.account";

class AppState {
	ready = $state(false);
	webConfig = $state<WebConfig | null>(null);
	/** Signed in (web) or an account is open (desktop). */
	signedIn = $state(false);
	account = $state<AccountInfo | null>(null);
	accounts = $state<AccountConfig[]>([]);
	/** Desktop: the account that needs its password typed. */
	passwordFor = $state<AccountConfig | null>(null);
	/** Desktop: the account editor is open (`null` id: new account). */
	editingAccount = $state<AccountConfig | null | undefined>(undefined);
	view = $state<View>("mail");
	online = $state(true);
	settings = $state<Settings>({ ...DEFAULT_SETTINGS });
	identities = $state<Identity[]>([]);
	/** Callbacks run after an account opens (the mail state loads folders). */
	private openHandlers: Array<() => void | Promise<void>> = [];

	get productName() {
		return this.webConfig?.productName ?? "LowMail";
	}

	onAccountOpen(handler: () => void | Promise<void>) {
		this.openHandlers.push(handler);
	}

	async init() {
		setSessionExpiredHandler(() => {
			if (this.signedIn) {
				this.signedIn = false;
				this.account = null;
				toast.info(t("auth.expired"));
			}
		});
		try {
			if (isDesktop) {
				this.accounts = await desktop.accounts();
				const last = localStorage.getItem(LAST_ACCOUNT_KEY);
				const first = this.accounts.find((a) => a.id === last) ?? this.accounts[0];
				if (first) await this.openAccount(first.id);
				else this.editingAccount = null;
			} else {
				this.webConfig = await web.config();
				try {
					const s = await web.session();
					this.account = s.account;
					await this.afterSignIn();
				} catch {
					this.signedIn = false;
				}
			}
		} catch (e) {
			toast.error(errorText(e));
		} finally {
			this.ready = true;
		}
	}

	async signIn(email: string, password: string) {
		const res = await web.login(email, password);
		this.account = res.account;
		await this.afterSignIn();
	}

	async signOut() {
		if (isDesktop) return;
		try {
			await web.logout();
		} finally {
			this.signedIn = false;
			this.account = null;
			location.hash = "";
		}
	}

	/** Desktop: opens an account, asking for its password when none is saved. */
	async openAccount(id: string, password: string | null = null) {
		const config = this.accounts.find((a) => a.id === id);
		try {
			this.account = await desktop.connect(id, password);
			this.passwordFor = null;
			setAccount(id);
			localStorage.setItem(LAST_ACCOUNT_KEY, id);
			await this.afterSignIn();
		} catch (e) {
			if (e instanceof ApiError && (e.kind === "auth" || e.message === "password-required")) {
				this.passwordFor = config ?? null;
				if (password !== null || e.kind === "auth") throw e;
				return;
			}
			throw e;
		}
	}

	private async afterSignIn() {
		if (!isDesktop) setAccount(this.account?.id ?? null);
		await this.loadSettings();
		this.signedIn = true;
		this.view = "mail";
		for (const h of this.openHandlers) await h();
	}

	async loadSettings() {
		try {
			const stored = await call<Partial<Settings>>("settings.get");
			this.settings = { ...DEFAULT_SETTINGS, ...stored, sort: { ...DEFAULT_SETTINGS.sort, ...(stored.sort ?? {}) } };
		} catch {
			this.settings = { ...DEFAULT_SETTINGS };
		}
		this.applySettings();
		await this.loadIdentities();
	}

	async loadIdentities() {
		try {
			this.identities = await call<Identity[]>("identities.list");
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	applySettings() {
		i18n.set(this.settings.language);
		colorScheme.setPreference(this.settings.theme);
		document.documentElement.dataset.density = this.settings.density;
	}

	async saveSettings(patch: Partial<Settings>) {
		this.settings = { ...this.settings, ...patch };
		this.applySettings();
		try {
			await call("settings.set", { settings: $state.snapshot(this.settings) });
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	get defaultIdentity(): Identity | undefined {
		return this.identities.find((i) => i.isDefault) ?? this.identities[0];
	}

	/** Every address of the account, to tell "me" apart in lists. */
	isMine(email: string) {
		const e = email.toLowerCase();
		return this.account?.email.toLowerCase() === e || this.identities.some((i) => i.email.toLowerCase() === e);
	}

	async reloadAccounts() {
		if (isDesktop) this.accounts = await desktop.accounts();
	}
}

export const app = new AppState();
