<template>
  <div class="dashboard">
    <header class="header glass">
      <div class="header-content">
        <div class="logo">
          <div class="logo-icon">⚡</div>
          <div>
            <h1>RustStrom</h1>
            <p class="subtitle">High-Performance Load Balancer</p>
          </div>
        </div>
        <div class="header-actions">
          <div class="status-badge" :class="{ 'status-online': isOnline, 'status-offline': !isOnline }">
            <span class="status-dot"></span>
            {{ isOnline ? 'Connected' : 'Disconnected' }}
          </div>
          <button class="btn btn-primary" @click="refreshData">
            <span>🔄</span> Refresh
          </button>
        </div>
      </div>
    </header>

    <div class="main-content">
      <nav class="sidebar glass">
        <div class="nav-item" :class="{ active: activeTab === 'overview' }" @click="activeTab = 'overview'">
          <span>📊</span> Overview
        </div>
        <div class="nav-item" :class="{ active: activeTab === 'metrics' }" @click="activeTab = 'metrics'">
          <span>📈</span> Metrics
        </div>
        <div class="nav-item" :class="{ active: activeTab === 'config' }" @click="activeTab = 'config'">
          <span>⚙️</span> Configuration
        </div>
        <div class="nav-item" :class="{ active: activeTab === 'backends' }" @click="activeTab = 'backends'">
          <span>🖥️</span> Backends
        </div>
      </nav>

      <main class="content">
        <OverviewTab v-if="activeTab === 'overview'" :metrics="metrics" :backends="backends" />
        <MetricsTab v-if="activeTab === 'metrics'" :metrics="metrics" :history="metricsHistory" />
        <ConfigTab v-if="activeTab === 'config'" :config="config" @update="updateConfig" @reload="reloadConfig" />
        <BackendsTab v-if="activeTab === 'backends'" :backends="backends" />
      </main>
    </div>
  </div>
</template>

<script>
import { ref, onMounted, onUnmounted } from 'vue'
import axios from 'axios'
import OverviewTab from './components/OverviewTab.vue'
import MetricsTab from './components/MetricsTab.vue'
import ConfigTab from './components/ConfigTab.vue'
import BackendsTab from './components/BackendsTab.vue'

export default {
  name: 'App',
  components: {
    OverviewTab,
    MetricsTab,
    ConfigTab,
    BackendsTab
  },
  setup() {
    const activeTab = ref('overview')
    const isOnline = ref(false)
    const metrics = ref({})
    const metricsHistory = ref([])
    const backends = ref([])
    const config = ref('')
    let refreshInterval = null

    const fetchMetrics = async () => {
      try {
        const response = await axios.get('/api/metrics')
        const metricsText = response.data
        const parsed = parsePrometheusMetrics(metricsText)
        metrics.value = parsed
        isOnline.value = true

        // Store history (keep last 60 data points = 5 minutes at 5s interval)
        metricsHistory.value.push({
          timestamp: Date.now(),
          ...parsed
        })
        if (metricsHistory.value.length > 60) {
          metricsHistory.value.shift()
        }
      } catch (error) {
        console.error('Failed to fetch metrics:', error)
        isOnline.value = false
      }
    }

    const fetchBackends = async () => {
      try {
        // Parse backends from metrics
        const response = await axios.get('/api/metrics')
        const metricsText = response.data
        const backendList = parseBackends(metricsText)
        backends.value = backendList
      } catch (error) {
        console.error('Failed to fetch backends:', error)
      }
    }

    const fetchConfig = async () => {
      try {
        const response = await axios.get('/api/config')
        config.value = response.data
      } catch (error) {
        console.error('Failed to fetch config:', error)
        // Fallback to placeholder
        config.value = `# RustStrom Configuration
# This is a sample config - replace with actual config endpoint

[[backend_pools]]
matcher = "Host('example.com')"
addresses = ["127.0.0.1:8080", "127.0.0.1:8081"]
schemes = ["HTTP", "HTTPS"]
strategy = { RoundRobin = {} }
`
      }
    }

    const parsePrometheusMetrics = (text) => {
      const lines = text.split('\n')
      const result = {}

      for (const line of lines) {
        if (line.startsWith('#') || !line.trim()) continue

        const match = line.match(/^([a-zA-Z_][a-zA-Z0-9_]*(?:{[^}]+})?) (.+)$/)
        if (match) {
          const [, key, value] = match
          result[key] = parseFloat(value)
        }
      }

      return result
    }

    const parseBackends = (text) => {
      const lines = text.split('\n')
      const backendMap = new Map()

      for (const line of lines) {
        if (line.includes('backend_pool=') && line.includes('backend_address=')) {
          const poolMatch = line.match(/backend_pool="([^"]+)"/)
          const addrMatch = line.match(/backend_address="([^"]+)"/)
          const statusMatch = line.match(/status="([^"]+)"/)

          if (poolMatch && addrMatch) {
            const pool = poolMatch[1]
            const addr = addrMatch[1]
            const status = statusMatch ? statusMatch[1] : 'unknown'
            const key = `${pool}:${addr}`

            if (!backendMap.has(key)) {
              backendMap.set(key, { pool, address: addr, status })
            }
          }
        }
      }

      return Array.from(backendMap.values())
    }

    const refreshData = async () => {
      await Promise.all([fetchMetrics(), fetchBackends(), fetchConfig()])
    }

    const updateConfig = async (newConfig) => {
      try {
        await axios.post('/api/config', newConfig, {
          headers: { 'Content-Type': 'text/plain' }
        })
        alert('Configuration updated successfully!')
      } catch (error) {
        alert('Failed to update configuration: ' + error.message)
      }
    }

    const reloadConfig = async () => {
      try {
        await axios.post('/api/reload')
        alert('Configuration reloaded successfully!')
        await refreshData()
      } catch (error) {
        alert('Failed to reload configuration: ' + error.message)
      }
    }

    onMounted(() => {
      refreshData()
      // Refresh every 5 seconds
      refreshInterval = setInterval(fetchMetrics, 5000)
    })

    onUnmounted(() => {
      if (refreshInterval) {
        clearInterval(refreshInterval)
      }
    })

    return {
      activeTab,
      isOnline,
      metrics,
      metricsHistory,
      backends,
      config,
      refreshData,
      updateConfig,
      reloadConfig
    }
  }
}
</script>

<style scoped>
.dashboard {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
}

.header {
  padding: 1.5rem 2rem;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  position: sticky;
  top: 0;
  z-index: 100;
}

.header-content {
  max-width: 1600px;
  margin: 0 auto;
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.logo {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.logo-icon {
  width: 48px;
  height: 48px;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 24px;
}

.logo h1 {
  font-size: 24px;
  font-weight: 700;
  color: #e6edf3;
  margin: 0;
}

.subtitle {
  font-size: 12px;
  color: #7d8590;
  margin: 0;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.status-badge {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 1rem;
  border-radius: 8px;
  font-size: 14px;
  font-weight: 500;
  background: rgba(255, 255, 255, 0.05);
}

.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.status-online .status-dot {
  background: #3fb950;
  box-shadow: 0 0 8px #3fb950;
  animation: pulse 2s ease-in-out infinite;
}

.status-offline .status-dot {
  background: #f85149;
}

.btn {
  padding: 0.625rem 1.25rem;
  border: none;
  border-radius: 8px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.btn-primary {
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
}

.btn-primary:hover {
  transform: translateY(-2px);
  box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
}

.main-content {
  flex: 1;
  display: flex;
  max-width: 1600px;
  width: 100%;
  margin: 0 auto;
  padding: 2rem;
  gap: 2rem;
}

.sidebar {
  width: 240px;
  height: fit-content;
  padding: 1rem;
  border-radius: 12px;
  position: sticky;
  top: 100px;
}

.nav-item {
  padding: 0.875rem 1rem;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.2s;
  display: flex;
  align-items: center;
  gap: 0.75rem;
  font-size: 14px;
  font-weight: 500;
  color: #7d8590;
  margin-bottom: 0.5rem;
}

.nav-item:hover {
  background: rgba(255, 255, 255, 0.05);
  color: #e6edf3;
}

.nav-item.active {
  background: linear-gradient(135deg, rgba(102, 126, 234, 0.2) 0%, rgba(118, 75, 162, 0.2) 100%);
  color: #e6edf3;
  border: 1px solid rgba(102, 126, 234, 0.3);
}

.content {
  flex: 1;
  animation: fadeIn 0.3s ease-out;
}
</style>
