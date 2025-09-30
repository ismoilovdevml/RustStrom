<template>
  <div class="metrics">
    <div class="chart-card glass fade-in">
      <div class="chart-header">
        <h3>Real-Time Performance</h3>
        <div class="chart-controls">
          <button class="btn-small" :class="{ active: timeRange === '1m' }" @click="timeRange = '1m'">1m</button>
          <button class="btn-small" :class="{ active: timeRange === '5m' }" @click="timeRange = '5m'">5m</button>
          <button class="btn-small" :class="{ active: timeRange === '15m' }" @click="timeRange = '15m'">15m</button>
        </div>
      </div>
      <div class="chart-container">
        <canvas ref="performanceChart"></canvas>
      </div>
    </div>

    <div class="metrics-grid">
      <!-- Request Metrics -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.1s">
        <h4>📊 Request Metrics</h4>
        <div class="metric-list">
          <div class="metric-item">
            <span class="metric-key">Total Requests</span>
            <span class="metric-value">{{ formatNumber(requestMetrics.total) }}</span>
          </div>
          <div class="metric-item">
            <span class="metric-key">Requests per Second</span>
            <span class="metric-value">{{ requestMetrics.rps.toFixed(2) }}/s</span>
          </div>
          <div class="metric-item">
            <span class="metric-key">Active Connections</span>
            <span class="metric-value">{{ requestMetrics.activeConnections }}</span>
          </div>
        </div>
      </div>

      <!-- Response Time Metrics -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.2s">
        <h4>⏱️ Response Time</h4>
        <div class="metric-list">
          <div class="metric-item">
            <span class="metric-key">Average Response</span>
            <span class="metric-value success">{{ responseMetrics.avgMs.toFixed(2) }}ms</span>
          </div>
          <div class="metric-item">
            <span class="metric-key">Total Requests</span>
            <span class="metric-value">{{ formatNumber(responseMetrics.totalRequests) }}</span>
          </div>
          <div class="metric-item">
            <span class="metric-key">Total Duration</span>
            <span class="metric-value">{{ responseMetrics.totalDuration.toFixed(3) }}s</span>
          </div>
        </div>
      </div>

      <!-- Error Metrics -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.3s">
        <h4>⚠️ Error Metrics</h4>
        <div class="metric-list" v-if="errorMetrics.hasErrors">
          <div class="metric-item">
            <span class="metric-key">Total Errors (5xx)</span>
            <span class="metric-value error">{{ errorMetrics.totalErrors }}</span>
          </div>
          <div class="metric-item">
            <span class="metric-key">Timeouts</span>
            <span class="metric-value error">{{ errorMetrics.timeouts }}</span>
          </div>
          <div class="metric-item">
            <span class="metric-key">Error Rate</span>
            <span class="metric-value error">{{ errorMetrics.errorRate.toFixed(2) }}%</span>
          </div>
        </div>
        <div v-else class="no-data">
          <span class="success-icon">✅</span>
          <p>No errors detected</p>
          <small>All requests successful</small>
        </div>
      </div>

      <!-- Backend Metrics -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.4s">
        <h4>🖥️ Backend Performance</h4>
        <div class="metric-list">
          <div v-for="backend in backendMetrics.backends" :key="backend.address" class="backend-metric">
            <div class="backend-header">
              <span class="backend-name">{{ backend.address }}</span>
              <span class="backend-status" :class="backend.status">{{ backend.status }}</span>
            </div>
            <div class="backend-stats">
              <div class="stat-item">
                <span class="stat-label">Requests</span>
                <span class="stat-value">{{ backend.requestCount }}</span>
              </div>
              <div class="stat-item">
                <span class="stat-label">Avg Time</span>
                <span class="stat-value">{{ backend.avgTime.toFixed(2) }}ms</span>
              </div>
              <div class="stat-item">
                <span class="stat-label">Failures</span>
                <span class="stat-value" :class="{ error: backend.failures > 0 }">{{ backend.failures }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Status Code Distribution -->
    <div class="chart-card glass fade-in" style="animation-delay: 0.5s">
      <h3>📈 HTTP Status Code Distribution</h3>
      <div class="status-grid">
        <div v-for="(count, code) in statusCodes" :key="code" class="status-item" :class="getStatusClass(code)">
          <div class="status-code">{{ code }}</div>
          <div class="status-count">{{ formatNumber(count) }}</div>
          <div class="status-label">{{ getStatusLabel(code) }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { Chart, registerables } from 'chart.js'

Chart.register(...registerables)

export default {
  name: 'MetricsTab',
  props: {
    metrics: {
      type: Object,
      required: true
    },
    history: {
      type: Array,
      required: true
    }
  },
  setup(props) {
    const performanceChart = ref(null)
    const timeRange = ref('5m')
    let chartInstance = null

    // Request Metrics - Clean and simple
    const requestMetrics = computed(() => {
      const total = props.metrics['http_requests_total'] || 0
      const rps = props.metrics['requests_per_second'] || 0
      const activeConnections = props.metrics['active_http_connections'] || 0

      return {
        total,
        rps,
        activeConnections
      }
    })

    // Response Time Metrics - Only important data
    const responseMetrics = computed(() => {
      const totalRequests = props.metrics['http_request_duration_seconds_count'] || 0
      const totalDuration = props.metrics['http_request_duration_seconds_sum'] || 0
      const avgMs = props.metrics['avg_response_time_ms'] || 0

      return {
        avgMs,
        totalRequests,
        totalDuration
      }
    })

    // Error Metrics - Show only if there are errors
    const errorMetrics = computed(() => {
      const totalErrors = props.metrics['http_errors_total'] || 0
      const timeouts = props.metrics['http_timeouts_total'] || 0
      const totalRequests = props.metrics['http_requests_total'] || 1
      const errorRate = (totalErrors / totalRequests) * 100

      return {
        totalErrors,
        timeouts,
        errorRate,
        hasErrors: totalErrors > 0 || timeouts > 0
      }
    })

    // Backend Metrics - Aggregated per backend
    const backendMetrics = computed(() => {
      const backends = []
      const backendAddresses = new Set()

      // Extract unique backend addresses
      Object.keys(props.metrics).forEach(key => {
        if (key.includes('backend_response_time_seconds_count{backend=')) {
          const match = key.match(/backend="([^"]+)"/)
          if (match) {
            backendAddresses.add(match[1])
          }
        }
      })

      // Build metrics for each backend
      backendAddresses.forEach(address => {
        const countKey = `backend_response_time_seconds_count{backend="${address}"}`
        const sumKey = `backend_response_time_seconds_sum{backend="${address}"}`
        const failuresKey = `backend_failures_total{backend="${address}"}`

        const requestCount = props.metrics[countKey] || 0
        const totalTime = props.metrics[sumKey] || 0
        const failures = props.metrics[failuresKey] || 0
        const avgTime = requestCount > 0 ? (totalTime / requestCount) * 1000 : 0

        backends.push({
          address,
          requestCount,
          avgTime,
          failures,
          status: failures > 0 ? 'unhealthy' : (avgTime > 100 ? 'slow' : 'healthy')
        })
      })

      return { backends }
    })

    // Status Codes
    const statusCodes = computed(() => {
      const codes = {}
      Object.keys(props.metrics).forEach(key => {
        if (key.includes('http_status_codes_total{code=')) {
          const match = key.match(/code="(\d+)"/)
          if (match) {
            codes[match[1]] = props.metrics[key]
          }
        }
      })
      return codes
    })

    const formatNumber = (num) => {
      if (num >= 1000000) return (num / 1000000).toFixed(1) + 'M'
      if (num >= 1000) return (num / 1000).toFixed(1) + 'K'
      return num.toString()
    }

    const getStatusClass = (code) => {
      const c = parseInt(code)
      if (c >= 200 && c < 300) return 'status-success'
      if (c >= 300 && c < 400) return 'status-redirect'
      if (c >= 400 && c < 500) return 'status-client-error'
      if (c >= 500) return 'status-server-error'
      return ''
    }

    const getStatusLabel = (code) => {
      const c = parseInt(code)
      if (c >= 200 && c < 300) return 'Success'
      if (c >= 300 && c < 400) return 'Redirect'
      if (c >= 400 && c < 500) return 'Client Error'
      if (c >= 500) return 'Server Error'
      return 'Unknown'
    }

    const updateChart = () => {
      if (!performanceChart.value) return

      const now = Date.now()
      const ranges = {
        '1m': 60 * 1000,
        '5m': 5 * 60 * 1000,
        '15m': 15 * 60 * 1000
      }

      const rangeMs = ranges[timeRange.value]
      const filteredHistory = props.history.filter(h => now - h.timestamp < rangeMs)

      const labels = filteredHistory.map(h => {
        const date = new Date(h.timestamp)
        return date.toLocaleTimeString()
      })

      const requestsData = filteredHistory.map(h => h.http_requests_total || 0)
      const responseTimeData = filteredHistory.map(h => h.avg_response_time_ms || 0)

      if (chartInstance) {
        chartInstance.data.labels = labels
        chartInstance.data.datasets[0].data = requestsData
        chartInstance.data.datasets[1].data = responseTimeData
        chartInstance.update('none')
      } else {
        chartInstance = new Chart(performanceChart.value, {
          type: 'line',
          data: {
            labels,
            datasets: [
              {
                label: 'Total Requests',
                data: requestsData,
                borderColor: 'rgba(102, 126, 234, 1)',
                backgroundColor: 'rgba(102, 126, 234, 0.1)',
                tension: 0.4,
                fill: true,
                yAxisID: 'y'
              },
              {
                label: 'Avg Response Time (ms)',
                data: responseTimeData,
                borderColor: 'rgba(240, 147, 251, 1)',
                backgroundColor: 'rgba(240, 147, 251, 0.1)',
                tension: 0.4,
                fill: true,
                yAxisID: 'y1'
              }
            ]
          },
          options: {
            responsive: true,
            maintainAspectRatio: false,
            interaction: {
              mode: 'index',
              intersect: false
            },
            plugins: {
              legend: {
                position: 'bottom',
                labels: {
                  color: '#e6edf3',
                  padding: 15,
                  font: {
                    size: 12
                  }
                }
              }
            },
            scales: {
              x: {
                grid: {
                  color: 'rgba(255, 255, 255, 0.05)'
                },
                ticks: {
                  color: '#7d8590'
                }
              },
              y: {
                type: 'linear',
                display: true,
                position: 'left',
                grid: {
                  color: 'rgba(255, 255, 255, 0.05)'
                },
                ticks: {
                  color: '#7d8590'
                }
              },
              y1: {
                type: 'linear',
                display: true,
                position: 'right',
                grid: {
                  drawOnChartArea: false
                },
                ticks: {
                  color: '#7d8590'
                }
              }
            }
          }
        })
      }
    }

    watch([() => props.history, timeRange], updateChart, { deep: true })
    onMounted(updateChart)

    onUnmounted(() => {
      if (chartInstance) {
        chartInstance.destroy()
      }
    })

    return {
      performanceChart,
      timeRange,
      requestMetrics,
      responseMetrics,
      errorMetrics,
      backendMetrics,
      statusCodes,
      formatNumber,
      getStatusClass,
      getStatusLabel
    }
  }
}
</script>

<style scoped>
.metrics {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.chart-card {
  padding: 1.5rem;
  border-radius: 12px;
}

.chart-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1.5rem;
}

.chart-header h3 {
  font-size: 18px;
  font-weight: 600;
  color: #e6edf3;
}

.chart-controls {
  display: flex;
  gap: 0.5rem;
}

.btn-small {
  padding: 0.375rem 0.875rem;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 6px;
  background: transparent;
  color: #7d8590;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s;
}

.btn-small:hover {
  background: rgba(255, 255, 255, 0.05);
  color: #e6edf3;
}

.btn-small.active {
  background: linear-gradient(135deg, rgba(102, 126, 234, 0.2) 0%, rgba(118, 75, 162, 0.2) 100%);
  border-color: rgba(102, 126, 234, 0.5);
  color: #e6edf3;
}

.chart-container {
  height: 400px;
}

.metrics-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  gap: 1.5rem;
}

.metric-card {
  padding: 1.5rem;
  border-radius: 12px;
}

.metric-card h4 {
  font-size: 16px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 1rem;
  padding-bottom: 0.75rem;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}

.metric-list {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.metric-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem;
  background: rgba(255, 255, 255, 0.02);
  border-radius: 6px;
  border: 1px solid rgba(255, 255, 255, 0.05);
}

.metric-key {
  font-size: 13px;
  color: #7d8590;
  font-weight: 500;
}

.metric-value {
  font-size: 16px;
  font-weight: 700;
  color: #e6edf3;
}

.metric-value.success {
  color: #3fb950;
}

.metric-value.error {
  color: #f85149;
}

.no-data {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 2rem;
  text-align: center;
}

.success-icon {
  font-size: 48px;
  margin-bottom: 0.5rem;
}

.no-data p {
  font-size: 14px;
  color: #3fb950;
  font-weight: 600;
  margin: 0.5rem 0 0.25rem 0;
}

.no-data small {
  font-size: 12px;
  color: #7d8590;
}

/* Backend Metrics */
.backend-metric {
  padding: 0.75rem;
  background: rgba(255, 255, 255, 0.02);
  border-radius: 6px;
  border: 1px solid rgba(255, 255, 255, 0.05);
  margin-bottom: 0.75rem;
}

.backend-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.75rem;
}

.backend-name {
  font-size: 13px;
  font-weight: 600;
  color: #e6edf3;
  font-family: 'Courier New', monospace;
}

.backend-status {
  font-size: 11px;
  font-weight: 600;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
  text-transform: uppercase;
}

.backend-status.healthy {
  background: rgba(63, 185, 80, 0.1);
  color: #3fb950;
}

.backend-status.slow {
  background: rgba(210, 153, 34, 0.1);
  color: #d29922;
}

.backend-status.unhealthy {
  background: rgba(248, 81, 73, 0.1);
  color: #f85149;
}

.backend-stats {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 0.5rem;
}

.stat-item {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.stat-label {
  font-size: 11px;
  color: #7d8590;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.stat-value {
  font-size: 14px;
  font-weight: 700;
  color: #e6edf3;
}

/* Status Code Grid */
.status-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
  gap: 1rem;
  margin-top: 1rem;
}

.status-item {
  padding: 1rem;
  border-radius: 8px;
  text-align: center;
  border: 2px solid;
  transition: transform 0.2s;
}

.status-item:hover {
  transform: translateY(-2px);
}

.status-success {
  background: rgba(63, 185, 80, 0.1);
  border-color: rgba(63, 185, 80, 0.3);
}

.status-redirect {
  background: rgba(79, 172, 254, 0.1);
  border-color: rgba(79, 172, 254, 0.3);
}

.status-client-error {
  background: rgba(240, 147, 251, 0.1);
  border-color: rgba(240, 147, 251, 0.3);
}

.status-server-error {
  background: rgba(248, 81, 73, 0.1);
  border-color: rgba(248, 81, 73, 0.3);
}

.status-code {
  font-size: 24px;
  font-weight: 700;
  margin-bottom: 0.25rem;
}

.status-success .status-code {
  color: #3fb950;
}

.status-redirect .status-code {
  color: #4facfe;
}

.status-client-error .status-code {
  color: #f093fb;
}

.status-server-error .status-code {
  color: #f85149;
}

.status-count {
  font-size: 18px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 0.25rem;
}

.status-label {
  font-size: 11px;
  color: #7d8590;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
</style>
