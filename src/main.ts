import "@fontsource-variable/inter";
import "@codeatlasdev/ui/styles/core.css";
import "./app.css";
import { colorScheme } from "@codeatlasdev/ui";
import { mount } from "svelte";
import App from "./App.svelte";

colorScheme.init();

mount(App, { target: document.getElementById("app")! });
