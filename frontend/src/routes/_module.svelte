<script lang="ts">
    import { Container } from "@svelteuidev/core"
    import Header from "@/components/Header.svelte"
    import CenterVoiceController from '@/components/CenterVoiceController.svelte'
</script>

<Container fluid id="wrapper">
    <Header />
    <CenterVoiceController />
    <!-- Every route mounts a fresh root element, so the CSS entrance below
         plays on each navigation without remounting anything by hand. -->
    <main class="route-stage">
        <slot></slot>
    </main>
</Container>

<style>
    .route-stage > :global(*) {
        animation: route-in .48s cubic-bezier(.22, 1, .36, 1) both;
    }
    @keyframes route-in {
        from { opacity: 0; transform: translateY(14px); filter: blur(3px); }
        to { opacity: 1; transform: none; filter: none; }
    }
    @media (prefers-reduced-motion: reduce) {
        .route-stage > :global(*) { animation: none; }
    }
</style>
