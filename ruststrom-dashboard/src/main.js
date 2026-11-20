import { createApp } from 'vue'
import axios from 'axios'
import App from './App.vue'
import './style.css'

// Custom header required by config-api.cjs; cross-site forms can not send it (CSRF protection)
axios.defaults.headers.common['X-RustStrom-Admin'] = '1'

createApp(App).mount('#app')
