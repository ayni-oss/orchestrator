import adapter from '@sveltejs/adapter-node';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';
export default defineConfig({plugins:[tailwindcss(),sveltekit({adapter:adapter(),alias:{$lib:'./src/lib'}})],server:{host:'127.0.0.1',port:Number(process.env.AYNI_ORCHESTRATOR_LIVE_UI_PORT||5174),strictPort:true,allowedHosts:['127.0.0.1'],cors:false}});
