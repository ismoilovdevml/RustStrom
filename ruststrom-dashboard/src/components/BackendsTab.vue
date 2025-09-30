<template>
  <div class="backends">
    <div class="backends-header glass fade-in">
      <div>
        <h3>Backend Servers</h3>
        <p class="backends-description">Monitor health and performance of backend servers</p>
      </div>
      <div class="filter-group">
        <button
          class="filter-btn"
          :class="{ active: filter === 'all' }"
          @click="filter = 'all'"
        >
          All ({{ backends.length }})
        </button>
        <button
          class="filter-btn"
          :class="{ active: filter === 'healthy' }"
          @click="filter = 'healthy'"
        >
          Healthy ({{ healthyCount }})
        </button>
        <button
          class="filter-btn"
          :class="{ active: filter === 'slow' }"
          @click="filter = 'slow'"
        >
          Slow ({{ slowCount }})
        </button>
        <button
          class="filter-btn"
          :class="{ active: filter === 'unhealthy' }"
          @click="filter = 'unhealthy'"
        >
          Unhealthy ({{ unhealthyCount }})
        </button>
      </div>
    </div>

    <div class="backends-grid">
      <div
        v-for="backend in filteredBackends"
        :key="backend.address"
        class="backend-card glass fade-in"
      >
        <div class="backend-card-header">
          <div class="backend-status-indicator" :class="getStatusClass(backend.status)"></div>
          <div class="backend-main-info">
            <h4 class="backend-address">{{ backend.address }}</h4>
            <p class="backend-pool-name">Pool: {{ backend.pool }}</p>
          </div>
          <div class="backend-status-badge" :class="getStatusClass(backend.status)">
            {{ backend.status }}
          </div>
        </div>

        <div class="backend-metrics">
          <div class="metric">
            <span class="metric-label">Requests</span>
            <span class="metric-value">{{ getBackendMetric(backend, 'requests') }}</span>
          </div>
          <div class="metric">
            <span class="metric-label">Response Time</span>
            <span class="metric-value">{{ getBackendMetric(backend, 'response_time') }}ms</span>
          </div>
          <div class="metric">
            <span class="metric-label">Success Rate</span>
            <span class="metric-value">{{ getBackendMetric(backend, 'success_rate') }}%</span>
          </div>
          <div class="metric">
            <span class="metric-label">Last Check</span>
            <span class="metric-value">{{ getBackendMetric(backend, 'last_check') }}</span>
          </div>
        </div>

        <div class="backend-actions">
          <button class="action-btn" @click="testBackend(backend)">
            <span>🔍</span> Test
          </button>
          <button class="action-btn" @click="viewLogs(backend)">
            <span>📋</span> Logs
          </button>
        </div>
      </div>
    </div>

    <div v-if="filteredBackends.length === 0" class="empty-state glass fade-in">
      <div class="empty-icon">🖥️</div>
      <h4>No backends found</h4>
      <p>No backends match the selected filter</p>
    </div>
  </div>
</template>

<script>
import { ref, computed } from 'vue'

export default {
  name: 'BackendsTab',
  props: {
    backends: {
      type: Array,
      required: true
    }
  },
  setup(props) {
    const filter = ref('all')

    const healthyCount = computed(() => {
      return props.backends.filter(b => b.status.toLowerCase() === 'healthy').length
    })

    const slowCount = computed(() => {
      return props.backends.filter(b => b.status.toLowerCase() === 'slow').length
    })

    const unhealthyCount = computed(() => {
      return props.backends.filter(b =>
        b.status.toLowerCase() !== 'healthy' && b.status.toLowerCase() !== 'slow'
      ).length
    })

    const filteredBackends = computed(() => {
      if (filter.value === 'all') return props.backends

      return props.backends.filter(b => {
        const status = b.status.toLowerCase()
        if (filter.value === 'healthy') return status === 'healthy'
        if (filter.value === 'slow') return status === 'slow'
        if (filter.value === 'unhealthy') return status !== 'healthy' && status !== 'slow'
        return true
      })
    })

    const getStatusClass = (status) => {
      const s = status.toLowerCase()
      if (s === 'healthy') return 'status-healthy'
      if (s === 'slow') return 'status-slow'
      return 'status-unhealthy'
    }

    const getBackendMetric = (backend, metric) => {
      // Placeholder metrics - in production, these would come from actual metrics
      switch (metric) {
        case 'requests':
          return Math.floor(Math.random() * 10000)
        case 'response_time':
          return backend.status.toLowerCase() === 'healthy'
            ? Math.floor(Math.random() * 50 + 20)
            : Math.floor(Math.random() * 200 + 100)
        case 'success_rate':
          return backend.status.toLowerCase() === 'healthy' ? 99.9 : 95.2
        case 'last_check':
          return 'Just now'
        default:
          return 'N/A'
      }
    }

    const testBackend = (backend) => {
      alert(`Testing backend: ${backend.address}\n\nThis would perform a health check...`)
    }

    const viewLogs = (backend) => {
      alert(`Viewing logs for: ${backend.address}\n\nLogs would be displayed here...`)
    }

    return {
      filter,
      healthyCount,
      slowCount,
      unhealthyCount,
      filteredBackends,
      getStatusClass,
      getBackendMetric,
      testBackend,
      viewLogs
    }
  }
}
</script>

<style scoped>
.backends {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.backends-header {
  padding: 1.5rem;
  border-radius: 12px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 1rem;
}

.backends-header h3 {
  font-size: 20px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 0.5rem;
}

.backends-description {
  font-size: 14px;
  color: #7d8590;
  margin: 0;
}

.filter-group {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.filter-btn {
  padding: 0.5rem 1rem;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 8px;
  background: transparent;
  color: #7d8590;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s;
}

.filter-btn:hover {
  background: rgba(255, 255, 255, 0.05);
  color: #e6edf3;
}

.filter-btn.active {
  background: linear-gradient(135deg, rgba(102, 126, 234, 0.2) 0%, rgba(118, 75, 162, 0.2) 100%);
  border-color: rgba(102, 126, 234, 0.5);
  color: #e6edf3;
}

.backends-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(350px, 1fr));
  gap: 1.5rem;
}

.backend-card {
  padding: 1.5rem;
  border-radius: 12px;
  transition: transform 0.2s;
}

.backend-card:hover {
  transform: translateY(-4px);
}

.backend-card-header {
  display: flex;
  align-items: flex-start;
  gap: 1rem;
  margin-bottom: 1.5rem;
  padding-bottom: 1rem;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}

.backend-status-indicator {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  flex-shrink: 0;
  margin-top: 0.25rem;
}

.backend-status-indicator.status-healthy {
  background: #3fb950;
  box-shadow: 0 0 12px #3fb950;
  animation: pulse 2s ease-in-out infinite;
}

.backend-status-indicator.status-slow {
  background: #d29922;
  box-shadow: 0 0 12px #d29922;
}

.backend-status-indicator.status-unhealthy {
  background: #f85149;
  box-shadow: 0 0 12px #f85149;
}

.backend-main-info {
  flex: 1;
}

.backend-address {
  font-size: 16px;
  font-weight: 600;
  color: #e6edf3;
  margin: 0 0 0.25rem 0;
}

.backend-pool-name {
  font-size: 13px;
  color: #7d8590;
  margin: 0;
}

.backend-status-badge {
  padding: 0.375rem 0.875rem;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.backend-status-badge.status-healthy {
  background: rgba(63, 185, 80, 0.15);
  color: #3fb950;
  border: 1px solid rgba(63, 185, 80, 0.3);
}

.backend-status-badge.status-slow {
  background: rgba(210, 153, 34, 0.15);
  color: #d29922;
  border: 1px solid rgba(210, 153, 34, 0.3);
}

.backend-status-badge.status-unhealthy {
  background: rgba(248, 81, 73, 0.15);
  color: #f85149;
  border: 1px solid rgba(248, 81, 73, 0.3);
}

.backend-metrics {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.metric {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.metric-label {
  font-size: 12px;
  color: #7d8590;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.metric-value {
  font-size: 16px;
  font-weight: 600;
  color: #e6edf3;
}

.backend-actions {
  display: flex;
  gap: 0.5rem;
}

.action-btn {
  flex: 1;
  padding: 0.625rem;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.02);
  color: #e6edf3;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
}

.action-btn:hover {
  background: rgba(255, 255, 255, 0.05);
  border-color: rgba(102, 126, 234, 0.5);
}

.empty-state {
  padding: 4rem 2rem;
  border-radius: 12px;
  text-align: center;
}

.empty-icon {
  font-size: 64px;
  margin-bottom: 1rem;
  opacity: 0.5;
}

.empty-state h4 {
  font-size: 18px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 0.5rem;
}

.empty-state p {
  font-size: 14px;
  color: #7d8590;
  margin: 0;
}
</style>
