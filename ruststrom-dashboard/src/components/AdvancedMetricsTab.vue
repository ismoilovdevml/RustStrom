<template>
  <div class="advanced-metrics">
    <div class="metrics-header glass fade-in">
      <div>
        <h3>Advanced Metrics</h3>
        <p class="metrics-description">Detailed performance analytics and monitoring</p>
      </div>
      <div class="metrics-actions">
        <select v-model="timeRange" class="time-range-select">
          <option value="5m">Last 5 minutes</option>
          <option value="15m">Last 15 minutes</option>
          <option value="1h">Last 1 hour</option>
          <option value="6h">Last 6 hours</option>
        </select>
        <button class="btn btn-primary" @click="refreshMetrics">
          <span>🔄</span> Refresh
        </button>
      </div>
    </div>

    <div class="metrics-grid">
      <!-- Request Rate Chart -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.1s">
        <div class="metric-card-header">
          <h4>Request Rate</h4>
          <span class="metric-badge">req/s</span>
        </div>
        <div class="chart-container">
          <canvas ref="requestRateChart"></canvas>
        </div>
      </div>

      <!-- Response Time Chart -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.15s">
        <div class="metric-card-header">
          <h4>Response Time</h4>
          <span class="metric-badge">ms</span>
        </div>
        <div class="chart-container">
          <canvas ref="responseTimeChart"></canvas>
        </div>
      </div>

      <!-- Error Rate Chart -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.2s">
        <div class="metric-card-header">
          <h4>Error Rate</h4>
          <span class="metric-badge">%</span>
        </div>
        <div class="chart-container">
          <canvas ref="errorRateChart"></canvas>
        </div>
      </div>

      <!-- Connection Pool -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.25s">
        <div class="metric-card-header">
          <h4>Connection Pool</h4>
          <span class="metric-badge">active</span>
        </div>
        <div class="chart-container">
          <canvas ref="connectionPoolChart"></canvas>
        </div>
      </div>

      <!-- Backend Performance -->
      <div class="metric-card glass fade-in full-width" style="animation-delay: 0.3s">
        <div class="metric-card-header">
          <h4>Backend Performance Comparison</h4>
          <span class="metric-badge">ms avg</span>
        </div>
        <div class="chart-container">
          <canvas ref="backendPerfChart"></canvas>
        </div>
      </div>

      <!-- Load Distribution -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.35s">
        <div class="metric-card-header">
          <h4>Load Distribution</h4>
          <span class="metric-badge">by backend</span>
        </div>
        <div class="chart-container">
          <canvas ref="loadDistChart"></canvas>
        </div>
      </div>

      <!-- HTTP Status Codes -->
      <div class="metric-card glass fade-in" style="animation-delay: 0.4s">
        <div class="metric-card-header">
          <h4>HTTP Status Codes</h4>
          <span class="metric-badge">distribution</span>
        </div>
        <div class="chart-container">
          <canvas ref="statusCodesChart"></canvas>
        </div>
      </div>
    </div>

    <!-- Detailed Stats Table -->
    <div class="stats-table glass fade-in" style="animation-delay: 0.45s">
      <h4>Detailed Statistics</h4>
      <table>
        <thead>
          <tr>
            <th>Metric</th>
            <th>Current</th>
            <th>Average</th>
            <th>Min</th>
            <th>Max</th>
            <th>P95</th>
            <th>P99</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><span class="metric-icon">⚡</span> Request Rate</td>
            <td>{{ detailedStats.requestRate.current }}</td>
            <td>{{ detailedStats.requestRate.avg }}</td>
            <td>{{ detailedStats.requestRate.min }}</td>
            <td>{{ detailedStats.requestRate.max }}</td>
            <td>{{ detailedStats.requestRate.p95 }}</td>
            <td>{{ detailedStats.requestRate.p99 }}</td>
          </tr>
          <tr>
            <td><span class="metric-icon">⏱️</span> Response Time (ms)</td>
            <td>{{ detailedStats.responseTime.current }}</td>
            <td>{{ detailedStats.responseTime.avg }}</td>
            <td>{{ detailedStats.responseTime.min }}</td>
            <td>{{ detailedStats.responseTime.max }}</td>
            <td>{{ detailedStats.responseTime.p95 }}</td>
            <td>{{ detailedStats.responseTime.p99 }}</td>
          </tr>
          <tr>
            <td><span class="metric-icon">❌</span> Error Rate (%)</td>
            <td>{{ detailedStats.errorRate.current }}</td>
            <td>{{ detailedStats.errorRate.avg }}</td>
            <td>{{ detailedStats.errorRate.min }}</td>
            <td>{{ detailedStats.errorRate.max }}</td>
            <td>{{ detailedStats.errorRate.p95 }}</td>
            <td>{{ detailedStats.errorRate.p99 }}</td>
          </tr>
          <tr>
            <td><span class="metric-icon">🔌</span> Active Connections</td>
            <td>{{ detailedStats.connections.current }}</td>
            <td>{{ detailedStats.connections.avg }}</td>
            <td>{{ detailedStats.connections.min }}</td>
            <td>{{ detailedStats.connections.max }}</td>
            <td>{{ detailedStats.connections.p95 }}</td>
            <td>{{ detailedStats.connections.p99 }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<script>
import { ref, onMounted, onUnmounted, watch } from 'vue'
import { Chart, registerables } from 'chart.js'

Chart.register(...registerables)

export default {
  name: 'AdvancedMetricsTab',
  props: {
    history: {
      type: Array,
      default: () => []
    }
  },
  setup(props) {
    const timeRange = ref('15m')
    const requestRateChart = ref(null)
    const responseTimeChart = ref(null)
    const errorRateChart = ref(null)
    const connectionPoolChart = ref(null)
    const backendPerfChart = ref(null)
    const loadDistChart = ref(null)
    const statusCodesChart = ref(null)

    const charts = ref({})

    const detailedStats = ref({
      requestRate: { current: 245, avg: 230, min: 180, max: 310, p95: 290, p99: 305 },
      responseTime: { current: 45, avg: 52, min: 32, max: 120, p95: 98, p99: 115 },
      errorRate: { current: 0.8, avg: 1.2, min: 0.2, max: 3.5, p95: 2.8, p99: 3.2 },
      connections: { current: 42, avg: 38, min: 15, max: 65, p95: 58, p99: 62 }
    })

    const chartOptions = {
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: {
          display: true,
          labels: {
            color: '#e6edf3',
            font: {
              size: 11
            }
          }
        }
      },
      scales: {
        x: {
          ticks: { color: '#7d8590' },
          grid: { color: 'rgba(255, 255, 255, 0.05)' }
        },
        y: {
          ticks: { color: '#7d8590' },
          grid: { color: 'rgba(255, 255, 255, 0.05)' }
        }
      }
    }

    const generateTimeLabels = () => {
      const labels = []
      for (let i = 14; i >= 0; i--) {
        labels.push(`${i}m ago`)
      }
      return labels
    }

    const generateRandomData = (min, max, count) => {
      return Array.from({ length: count }, () => Math.floor(Math.random() * (max - min + 1)) + min)
    }

    const initCharts = () => {
      const timeLabels = generateTimeLabels()

      // Request Rate Chart
      if (requestRateChart.value) {
        charts.value.requestRate = new Chart(requestRateChart.value, {
          type: 'line',
          data: {
            labels: timeLabels,
            datasets: [{
              label: 'Requests/sec',
              data: generateRandomData(180, 310, 15),
              borderColor: '#667eea',
              backgroundColor: 'rgba(102, 126, 234, 0.1)',
              fill: true,
              tension: 0.4
            }]
          },
          options: chartOptions
        })
      }

      // Response Time Chart
      if (responseTimeChart.value) {
        charts.value.responseTime = new Chart(responseTimeChart.value, {
          type: 'line',
          data: {
            labels: timeLabels,
            datasets: [
              {
                label: 'Avg Response (ms)',
                data: generateRandomData(35, 90, 15),
                borderColor: '#43e97b',
                backgroundColor: 'rgba(67, 233, 123, 0.1)',
                fill: true,
                tension: 0.4
              },
              {
                label: 'P95 Response (ms)',
                data: generateRandomData(80, 120, 15),
                borderColor: '#d29922',
                backgroundColor: 'rgba(210, 153, 34, 0.1)',
                fill: false,
                tension: 0.4
              }
            ]
          },
          options: chartOptions
        })
      }

      // Error Rate Chart
      if (errorRateChart.value) {
        charts.value.errorRate = new Chart(errorRateChart.value, {
          type: 'line',
          data: {
            labels: timeLabels,
            datasets: [{
              label: 'Error %',
              data: generateRandomData(0, 4, 15).map(v => v / 10),
              borderColor: '#f85149',
              backgroundColor: 'rgba(248, 81, 73, 0.1)',
              fill: true,
              tension: 0.4
            }]
          },
          options: chartOptions
        })
      }

      // Connection Pool Chart
      if (connectionPoolChart.value) {
        charts.value.connectionPool = new Chart(connectionPoolChart.value, {
          type: 'line',
          data: {
            labels: timeLabels,
            datasets: [{
              label: 'Active Connections',
              data: generateRandomData(15, 65, 15),
              borderColor: '#58a6ff',
              backgroundColor: 'rgba(88, 166, 255, 0.1)',
              fill: true,
              tension: 0.4
            }]
          },
          options: chartOptions
        })
      }

      // Backend Performance Chart
      if (backendPerfChart.value) {
        charts.value.backendPerf = new Chart(backendPerfChart.value, {
          type: 'bar',
          data: {
            labels: ['Backend 1\n:8080', 'Backend 2\n:8081', 'Backend 3\n:8082'],
            datasets: [{
              label: 'Avg Response Time (ms)',
              data: [45, 52, 48],
              backgroundColor: [
                'rgba(102, 126, 234, 0.6)',
                'rgba(67, 233, 123, 0.6)',
                'rgba(88, 166, 255, 0.6)'
              ],
              borderColor: [
                '#667eea',
                '#43e97b',
                '#58a6ff'
              ],
              borderWidth: 2
            }]
          },
          options: chartOptions
        })
      }

      // Load Distribution Chart
      if (loadDistChart.value) {
        charts.value.loadDist = new Chart(loadDistChart.value, {
          type: 'doughnut',
          data: {
            labels: [':8080', ':8081', ':8082'],
            datasets: [{
              data: [35, 33, 32],
              backgroundColor: [
                'rgba(102, 126, 234, 0.8)',
                'rgba(67, 233, 123, 0.8)',
                'rgba(88, 166, 255, 0.8)'
              ],
              borderColor: [
                '#667eea',
                '#43e97b',
                '#58a6ff'
              ],
              borderWidth: 2
            }]
          },
          options: {
            ...chartOptions,
            plugins: {
              legend: {
                position: 'bottom',
                labels: {
                  color: '#e6edf3',
                  font: { size: 11 }
                }
              }
            }
          }
        })
      }

      // HTTP Status Codes Chart
      if (statusCodesChart.value) {
        charts.value.statusCodes = new Chart(statusCodesChart.value, {
          type: 'doughnut',
          data: {
            labels: ['2xx Success', '3xx Redirect', '4xx Client Error', '5xx Server Error'],
            datasets: [{
              data: [92, 5, 2, 1],
              backgroundColor: [
                'rgba(67, 233, 123, 0.8)',
                'rgba(88, 166, 255, 0.8)',
                'rgba(210, 153, 34, 0.8)',
                'rgba(248, 81, 73, 0.8)'
              ],
              borderColor: [
                '#43e97b',
                '#58a6ff',
                '#d29922',
                '#f85149'
              ],
              borderWidth: 2
            }]
          },
          options: {
            ...chartOptions,
            plugins: {
              legend: {
                position: 'bottom',
                labels: {
                  color: '#e6edf3',
                  font: { size: 11 }
                }
              }
            }
          }
        })
      }
    }

    const refreshMetrics = () => {
      // Refresh all charts with new data
      Object.values(charts.value).forEach(chart => {
        if (chart && chart.data && chart.data.datasets) {
          chart.data.datasets.forEach(dataset => {
            if (dataset.data && Array.isArray(dataset.data)) {
              const len = dataset.data.length
              if (chart.config.type === 'doughnut') {
                dataset.data = dataset.data.map(() => Math.floor(Math.random() * 40) + 20)
              } else {
                dataset.data = generateRandomData(
                  Math.min(...dataset.data) - 10,
                  Math.max(...dataset.data) + 10,
                  len
                )
              }
            }
          })
          chart.update()
        }
      })
    }

    onMounted(() => {
      initCharts()
    })

    onUnmounted(() => {
      Object.values(charts.value).forEach(chart => {
        if (chart) chart.destroy()
      })
    })

    return {
      timeRange,
      requestRateChart,
      responseTimeChart,
      errorRateChart,
      connectionPoolChart,
      backendPerfChart,
      loadDistChart,
      statusCodesChart,
      detailedStats,
      refreshMetrics
    }
  }
}
</script>

<style scoped>
.advanced-metrics {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.metrics-header {
  padding: 1.5rem;
  border-radius: 12px;
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.metrics-header h3 {
  font-size: 20px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 0.5rem;
}

.metrics-description {
  font-size: 14px;
  color: #7d8590;
  margin: 0;
}

.metrics-actions {
  display: flex;
  gap: 0.75rem;
  align-items: center;
}

.time-range-select {
  padding: 0.625rem 1rem;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 8px;
  color: #e6edf3;
  font-size: 14px;
  cursor: pointer;
  outline: none;
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

.metrics-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
  gap: 1.5rem;
}

.metric-card {
  padding: 1.5rem;
  border-radius: 12px;
  display: flex;
  flex-direction: column;
}

.metric-card.full-width {
  grid-column: 1 / -1;
}

.metric-card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

.metric-card-header h4 {
  font-size: 16px;
  font-weight: 600;
  color: #e6edf3;
}

.metric-badge {
  padding: 0.25rem 0.75rem;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 12px;
  font-size: 11px;
  color: #7d8590;
  font-weight: 500;
}

.chart-container {
  position: relative;
  height: 250px;
  width: 100%;
}

.stats-table {
  padding: 1.5rem;
  border-radius: 12px;
}

.stats-table h4 {
  font-size: 16px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 1rem;
}

.stats-table table {
  width: 100%;
  border-collapse: collapse;
}

.stats-table th {
  text-align: left;
  padding: 0.75rem 1rem;
  font-size: 13px;
  font-weight: 600;
  color: #7d8590;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}

.stats-table td {
  padding: 0.75rem 1rem;
  font-size: 13px;
  color: #e6edf3;
  border-bottom: 1px solid rgba(255, 255, 255, 0.03);
}

.stats-table tr:hover {
  background: rgba(255, 255, 255, 0.02);
}

.metric-icon {
  margin-right: 0.5rem;
}
</style>
