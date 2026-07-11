import { mount } from "svelte";
import FloatingLyric from "./FloatingLyric.svelte";
import "./float.css";

mount(FloatingLyric, { target: document.getElementById("float-app")! });
