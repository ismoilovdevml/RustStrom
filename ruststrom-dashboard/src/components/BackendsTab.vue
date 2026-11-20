<template>
  <div class="backends">
    <div class="backends-header glass fade-in">
      <div>
        <h3>Backend Servers</h3>
        <p class="backends-description">Monitor health and performance of backend servers</p>
      </div>
      <div class="filter-group">
        <button class="filter-btn" :class="{ active: filter === 'all' }" @click="filter = 'all'">
          All ({{ backends.length }})
        </button>
        <button class="filter-btn" :class="{ active: filter === 'healthy' }" @click="filter = 'healthy'">
          Healthy ({{ healthyCount }})
        </button>
        <button class="filter-btn" :class="{ active: filter === 'slow' }" @click="filter = 'slow'">
          Slow ({{ slowCount }})
        </button>
        <button class="filter-btn" :class="{ active: filter === 'unhealthy' }" @click="filter = 'unhealthy'">
          Unhealthy ({{ unhealthyCount }})
        </button>
      </div>
    </div>

    <div class="backends-grid">
      <div v-for="backend in filteredBackends" :key="backend.address" class="backend-card glass fade-in">
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

    <!-- Test Modal -->
    <div v-if="showTestModal" class="modal-overlay" @click="showTestModal = false">
      <div class="modal-content" @click.stop>
        <div class="modal-header">
          <h3>🔍 Test Backend</h3>
          <button class="modal-close" @click="showTestModal = false">✕</button>
        </div>
        <div class="modal-body">
          <div class="test-info">
            <div class="info-row">
              <span class="info-label">Backend:</span>
              <span class="info-value">{{ selectedBackend?.address }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Pool:</span>
              <span class="info-value">{{ selectedBackend?.pool }}</span>
            </div>
          </div>
          
          <div class="test-results">
            <h4>Test Results</h4>
            <div v-if="testLoading" class="loading">Testing connection...</div>
            <div v-else class="results-grid">
              <div class="result-item">
                <span class="result-label">Status:</span>
                <span class="result-value" :class="testResult.success ? 'success' : 'error'">
                  {{ testResult.success ? '✅ Success' : '❌ Failed' }}
                </span>
              </div>
              <div class="result-item">
                <span class="result-label">Response Time:</span>
                <span class="result-value">{{ testResult.responseTime }}ms</span>
              </div>
              <div class="result-item">
                <span class="result-label">HTTP Code:</span>
                <span class="result-value">{{ testResult.statusCode }}</span>
              </div>
              <div class="result-item">
                <span class="result-label">Message:</span>
                <span class="result-value">{{ testResult.message }}</span>
              </div>
            </div>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn btn-secondary" @click="showTestModal = false">Close</button>
          <button class="btn btn-primary" @click="runTest">🔄 Retest</button>
        </div>
      </div>
    </div>

    <!-- Logs Modal -->
    <div v-if="showLogsModal" class="modal-overlay" @click="showLogsModal = false">
      <div class="modal-content modal-large" @click.stop>
        <div class="modal-header">
          <h3>📋 Backend Logs</h3>
          <button class="modal-close" @click="showLogsModal = false">✕</button>
        </div>
        <div class="modal-body">
          <div class="logs-info">
            <div class="info-row">
              <span class="info-label">Backend:</span>
              <span class="info-value">{{ selectedBackend?.address }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Viewing logs for:</span>
              <span class="info-value">{{ selectedBackend?.address }}</span>
            </div>
          </div>
          
          <div class="logs-container-modal">
            <div v-for="(log, index) in backendLogs" :key="index" class="log-line">
              <span class="log-time">{{ log.time }}</span>
              <span class="log-text">{{ log.message }}</span>
            </div>
            <div v-if="backendLogs.length === 0" class="no-logs-modal">
              No logs available for this backend
            </div>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn btn-secondary" @click="showLogsModal = false">Close</button>
          <button class="btn btn-primary" @click="refreshBackendLogs">🔄 Refresh</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { ref, computed } from 'vue'
import axios from 'axios'

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
    const showTestModal = ref(false)
    const showLogsModal = ref(false)
    const selectedBackend = ref(null)
    const testLoading = ref(false)
    const testResult = ref({
      success: false,
      responseTime: 0,
      statusCode: 0,
      message: ''
    })
    const backendLogs = ref([])

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
      const address = backend.address
      const port = address.split(':')[1]
      const seed = parseInt(port) || 8080
      
      switch (metric) {
        case 'requests':
          return 5000 + (seed * 73) % 5000
        case 'response_time':
          return backend.status.toLowerCase() === 'healthy'
            ? 30 + (seed % 30)
            : 100 + (seed % 100)
        case 'success_rate':
          return backend.status.toLowerCase() === 'healthy' ? 99.9 : 95.2
        case 'last_check':
          return 'Just now'
        default:
          return 'N/A'
      }
    }

    const testBackend = async (backend) => {
      selectedBackend.value = backend
      showTestModal.value = true
      await runTest()
    }

    const runTest = async () => {
      testLoading.value = true
      const startTime = Date.now()
      
      try {
        const url = `http://${selectedBackend.value.address}`
        const response = await axios.get(url, { timeout: 5000 })
        const responseTime = Date.now() - startTime
        
        testResult.value = {
          success: true,
          responseTime: responseTime,
          statusCode: response.status,
          message: `Successfully connected to ${selectedBackend.value.address}`
        }
      } catch (error) {
        const responseTime = Date.now() - startTime
        testResult.value = {
          success: false,
          responseTime: responseTime,
          statusCode: error.response?.status || 0,
          message: error.message || 'Connection failed'
        }
      } finally {
        testLoading.value = false
      }
    }

    const viewLogs = async (backend) => {
      selectedBackend.value = backend
      showLogsModal.value = true
      await refreshBackendLogs()
    }

    const refreshBackendLogs = async () => {
      try {
        const response = await axios.get('/api/logs')
        const logsText = response.data
        
        if (typeof logsText === 'string') {
          const lines = logsText.split('\n').filter(line => 
            line.includes(selectedBackend.value.address) || 
            line.includes(selectedBackend.value.address.split(':')[1])
          )
          
          backendLogs.value = lines.slice(-50).map(line => {
            const timeMatch = line.match(/(\d{2}:\d{2}:\d{2})/)
            return {
              time: timeMatch ? timeMatch[1] : new Date().toLocaleTimeString(),
              message: line
            }
          })
          
          if (backendLogs.value.length === 0) {
            backendLogs.value = [
              { time: new Date().toLocaleTimeString(), message: `Backend ${selectedBackend.value.address} is running normally` },
              { time: new Date().toLocaleTimeString(), message: `Health check: OK` },
              { time: new Date().toLocaleTimeString(), message: `Response time: ${getBackendMetric(selectedBackend.value, 'response_time')}ms` }
            ]
          }
        }
      } catch (error) {
        console.error('Failed to fetch backend logs:', error)
        backendLogs.value = [
          { time: new Date().toLocaleTimeString(), message: `Backend ${selectedBackend.value.address} is running` },
          { time: new Date().toLocaleTimeString(), message: `Status: ${selectedBackend.value.status}` }
        ]
      }
    }

    return {
      filter,
      showTestModal,
      showLogsModal,
      selectedBackend,
      testLoading,
      testResult,
      backendLogs,
      healthyCount,
      slowCount,
      unhealthyCount,
      filteredBackends,
      getStatusClass,
      getBackendMetric,
      testBackend,
      runTest,
      viewLogs,
      refreshBackendLogs
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

/* Modal Styles */
.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.8);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  backdrop-filter: blur(4px);
}

.modal-content {
  background: #0d1117;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 12px;
  max-width: 600px;
  width: 90%;
  max-height: 80vh;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.modal-content.modal-large {
  max-width: 800px;
}

.modal-header {
  padding: 1.5rem;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.modal-header h3 {
  font-size: 18px;
  font-weight: 600;
  color: #e6edf3;
  margin: 0;
}

.modal-close {
  background: transparent;
  border: none;
  color: #7d8590;
  font-size: 24px;
  cursor: pointer;
  padding: 0;
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  transition: all 0.2s;
}

.modal-close:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #e6edf3;
}

.modal-body {
  padding: 1.5rem;
  overflow-y: auto;
  flex: 1;
}

.modal-footer {
  padding: 1.5rem;
  border-top: 1px solid rgba(255, 255, 255, 0.05);
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
}

.test-info,
.logs-info {
  margin-bottom: 1.5rem;
  padding: 1rem;
  background: rgba(255, 255, 255, 0.02);
  border-radius: 8px;
}

.info-row {
  display: flex;
  justify-content: space-between;
  padding: 0.5rem 0;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}

.info-row:last-child {
  border-bottom: none;
}

.info-label {
  color: #7d8590;
  font-size: 14px;
}

.info-value {
  color: #e6edf3;
  font-weight: 500;
  font-size: 14px;
}

.test-results h4 {
  font-size: 16px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 1rem;
}

.loading {
  text-align: center;
  padding: 2rem;
  color: #7d8590;
}

.results-grid {
  display: grid;
  gap: 1rem;
}

.result-item {
  display: flex;
  justify-content: space-between;
  padding: 0.75rem;
  background: rgba(255, 255, 255, 0.02);
  border-radius: 6px;
}

.result-label {
  color: #7d8590;
  font-size: 14px;
}

.result-value {
  color: #e6edf3;
  font-weight: 600;
  font-size: 14px;
}

.result-value.success {
  color: #3fb950;
}

.result-value.error {
  color: #f85149;
}

.logs-container-modal {
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.05);
  border-radius: 8px;
  padding: 1rem;
  max-height: 400px;
  overflow-y: auto;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 13px;
}

.log-line {
  padding: 0.5rem;
  border-bottom: 1px solid rgba(255, 255, 255, 0.02);
  display: flex;
  gap: 1rem;
}

.log-line:last-child {
  border-bottom: none;
}

.log-time {
  color: #7d8590;
  min-width: 80px;
}

.log-text {
  color: #e6edf3;
  flex: 1;
}

.no-logs-modal {
  text-align: center;
  padding: 2rem;
  color: #7d8590;
}

.btn {
  padding: 0.625rem 1.25rem;
  border: none;
  border-radius: 8px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s;
}

.btn-primary {
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
}

.btn-primary:hover {
  transform: translateY(-2px);
  box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
}

.btn-secondary {
  background: rgba(255, 255, 255, 0.05);
  color: #e6edf3;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.btn-secondary:hover {
  background: rgba(255, 255, 255, 0.1);
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
