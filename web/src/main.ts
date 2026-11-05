import { createApp } from 'vue'
import './style.css'
import App from './App.vue'
import SettingsApp from './settings/SettingsApp.vue'

createApp(location.hash.startsWith('#/settings/') ? SettingsApp : App).mount('#app')
