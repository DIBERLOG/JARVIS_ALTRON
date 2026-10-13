// ### STYLES
import "./css/main.scss"
import "./css/styles.scss"

// ### APP
import "./lib/appearance"
import "./css/appearance.css"
import App from "./App.svelte"

const app = new App({
    target: document.getElementById("app")!
})

export default app
