<script lang="ts">
    /**
     * The shell every page is drawn into.
     *
     * The wrapper fades and lifts slightly while the first paint happens: 200 ms,
     * no library, and nothing at all when the person asked for reduced motion. It is
     * decoration on top of a working window — a backend error is still shown, and no
     * delay is added to anything.
     */
    import { Container } from "@svelteuidev/core"
    import Header from "@/components/Header.svelte"
</script>

<div class="app-shell">
    <Container fluid id="wrapper">
        <Header />
        <main>
            <slot></slot>
        </main>
    </Container>
</div>

<style>
    .app-shell {
        animation: jarvis-appear 200ms ease-out both;
    }

    @keyframes jarvis-appear {
        from {
            opacity: 0;
            transform: translateY(8px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }

    @media (prefers-reduced-motion: reduce) {
        .app-shell {
            animation: none;
            opacity: 1;
            transform: none;
        }
    }
</style>
