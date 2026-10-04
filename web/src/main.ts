import { mount } from "svelte";
import "./styles/report.css";
import "./styles/app.css";
import App from "./App.svelte";

mount(App, { target: document.getElementById("app")! });
