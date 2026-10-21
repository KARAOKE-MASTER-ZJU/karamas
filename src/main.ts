import "./style.css";

const app = document.querySelector<HTMLDivElement>("#app");

if (!app) {
  throw new Error("Application root was not found.");
}

const main = document.createElement("main");
const title = document.createElement("h1");
title.textContent = "KTV Casting";
main.append(title);
app.replaceChildren(main);
