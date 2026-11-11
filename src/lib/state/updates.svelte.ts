/**
 * Automatic updates from GitHub Releases. The app checks shortly after it
 * opens, every few minutes and whenever the window regains focus. A copy that
 * can update itself (Windows installer, macOS app, writable AppImage, .deb or
 * .rpm) downloads the signed update in the background; once it is verified and
 * ready, the rail offers "restart to update", which installs it right away.
 * Other formats (Arch package, portable copy) get a download link.
 */
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { isDesktop, isMobile, openExternal } from "../api";
import { errorText, t } from "../i18n/index.svelte";

export interface UpdatePolicy {
	/** installer, appimage, deb, rpm, package, portable or mobile. */
	channel: string;
	canInstall: boolean;
	target: string | null;
}

const RELEASES_URL = "https://github.com/LowBloat/LowMail/releases/latest";
const FIRST_CHECK_MS = 5_000;
const CHECK_EVERY_MS = 10 * 60 * 1000;
/** Focus comes and goes quickly; at most one extra check per this long. */
const FOCUS_GAP_MS = 2 * 60 * 1000;

class Updates {
	currentVersion = $state("");
	policy = $state<UpdatePolicy | null>(null);
	available = $state<{ version: string; notes: string } | null>(null);
	checking = $state(false);
	/** Downloading in the background, before anyone asked. */
	downloading = $state(false);
	/** Downloaded and verified: a restart installs it. */
	ready = $state(false);
	installing = $state(false);
	/** Download progress from 0 to 1, or null when the size is unknown. */
	progress = $state<number | null>(null);
	error = $state<string | null>(null);
	dialogOpen = $state(false);
	/** Set after a check the user asked for found nothing new. */
	upToDate = $state(false);
	private update: Update | null = null;
	private timer: ReturnType<typeof setInterval> | undefined;
	private lastCheck = 0;
	private prefetching: Promise<void> | null = null;

	/** Self-update exists in the desktop app only. */
	get supported() {
		return isDesktop && !isMobile;
	}

	/** The rail and Settings offer the update once it is ready (or when it is manual). */
	get offered(): boolean {
		return this.available !== null && !this.downloading;
	}

	/** "Restart to update" once downloaded, "Update to" otherwise. */
	get label(): string {
		if (!this.available) return "";
		return t(this.ready ? "updates.ready" : "updates.available", { version: this.available.version });
	}

	/** Why this copy cannot update itself, shown next to the download link. */
	get manualHint(): string | null {
		if (!this.policy || this.policy.canInstall) return null;
		const known = ["package", "appimage"];
		return t(`updates.manual.${known.includes(this.policy.channel) ? this.policy.channel : "portable"}`);
	}

	/** Starts background checks; safe to call once at startup. */
	start() {
		if (!this.supported || this.timer) return;
		getVersion()
			.then((v) => (this.currentVersion = v))
			.catch(() => {});
		setTimeout(() => this.check(false), FIRST_CHECK_MS);
		this.timer = setInterval(() => this.check(false), CHECK_EVERY_MS);
		window.addEventListener("focus", () => {
			if (Date.now() - this.lastCheck > FOCUS_GAP_MS) void this.check(false);
		});
	}

	/** `interactive`: the user asked, so report "up to date" and errors. */
	async check(interactive: boolean): Promise<void> {
		if (!this.supported || this.checking || this.downloading || this.installing) return;
		this.checking = true;
		this.lastCheck = Date.now();
		if (interactive) this.error = null;
		this.upToDate = false;
		try {
			this.policy ??= await invoke<UpdatePolicy>("update_policy");
			// Copies that cannot install still read the AppImage entry to learn
			// the version and notes.
			const target = this.policy.target ?? (this.policy.canInstall ? undefined : "linux-x86_64-appimage");
			const update = await check(target ? { target } : undefined);
			if (!update) {
				this.update = null;
				this.available = null;
				this.ready = false;
				if (interactive) this.upToDate = true;
				return;
			}
			// Already downloaded this one: nothing to do until the restart.
			if (this.ready && this.available?.version === update.version) {
				if (interactive) this.dialogOpen = true;
				return;
			}
			this.update = update;
			this.ready = false;
			this.available = { version: update.version, notes: update.body ?? "" };
			if (interactive) this.dialogOpen = true;
			if (this.policy.canInstall) this.prefetching = this.prefetch(update);
		} catch {
			// GitHub unreachable, or no release yet: the updater's own text is technical.
			if (interactive) this.error = t("updates.checkFailed");
		} finally {
			this.checking = false;
		}
	}

	/** Downloads and verifies in the background; the rail changes once it is ready. */
	private async prefetch(update: Update): Promise<void> {
		this.downloading = true;
		this.progress = null;
		let total = 0;
		let received = 0;
		try {
			await update.download((event) => {
				if (event.event === "Started") total = event.data.contentLength ?? 0;
				else if (event.event === "Progress") {
					received += event.data.chunkLength;
					this.progress = total > 0 ? Math.min(1, received / total) : null;
				}
			});
			if (this.update === update) this.ready = true;
		} catch {
			// The dialog falls back to download-and-install in one go.
		} finally {
			this.downloading = false;
			this.prefetching = null;
			if (!this.installing) this.progress = null;
		}
	}

	async install(): Promise<void> {
		const update = this.update;
		if (!update || !this.policy?.canInstall || this.installing) return;
		this.installing = true;
		this.error = null;
		try {
			// Clicked while the background download runs: let it finish first.
			if (this.prefetching) await this.prefetching;
			if (this.ready) {
				this.progress = 1;
				await update.install();
			} else {
				this.progress = null;
				let total = 0;
				let received = 0;
				await update.downloadAndInstall((event) => {
					if (event.event === "Started") total = event.data.contentLength ?? 0;
					else if (event.event === "Progress") {
						received += event.data.chunkLength;
						this.progress = total > 0 ? Math.min(1, received / total) : null;
					} else if (event.event === "Finished") this.progress = 1;
				});
			}
			await relaunch();
		} catch (e) {
			this.error = errorText(e);
			this.installing = false;
		}
	}

	download() {
		void openExternal(RELEASES_URL);
	}
}

export const updates = new Updates();
