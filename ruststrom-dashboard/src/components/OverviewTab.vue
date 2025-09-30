<template>
  <div class="overview">
    <div class="stats-grid">
      <div class="stat-card glass fade-in">
        <div class="stat-icon" style="background: linear-gradient(135deg, #667eea 0%, #764ba2 100%)">📊</div>
        <div class="stat-content">
          <div class="stat-label">Total Requests</div>
          <div class="stat-value">{{ formatNumber(totalRequests) }}</div>
          <div class="stat-trend positive">↗ +{{ requestsPerSecond.toFixed(1) }}/s</div>
        </div>
      </div>

      <div class="stat-card glass fade-in" style="animation-delay: 0.1s">
        <div class="stat-icon" style="background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%)">⏱️</div>
        <div class="stat-content">
          <div class="stat-label">Avg Response Time</div>
          <div class="stat-value">{{ avgResponseTime }}ms</div>
          <div class="stat-trend" :class="{ positive: avgResponseTime < 100, negative: avgResponseTime >= 100 }">
            {{ avgResponseTime < 100 ? '↗' : '↘' }} {{ avgResponseTime < 100 ? 'Fast' : 'Slow' }}
          </div>
        </div>
      </div>

      <div class="stat-card glass fade-in" style="animation-delay: 0.2s">
        <div class="stat-icon" style="background: linear-gradient(135deg, #4facfe 0%, #00f2fe 100%)">✅</div>
        <div class="stat-content">
          <div class="stat-label">Success Rate</div>
          <div class="stat-value">{{ successRate }}%</div>
          <div class="stat-trend" :class="{ positive: successRate >= 99, negative: successRate < 99 }">
            {{ successRate >= 99 ? '↗' : '↘' }} {{ successRate >= 99 ? 'Excellent' : 'Warning' }}
          </div>
        </div>
      </div>

      <div class="stat-card glass fade-in" style="animation-delay: 0.3s">
        <div class="stat-icon" style="background: linear-gradient(135deg, #43e97b 0%, #38f9d7 100%)">🖥️</div>
        <div class="stat-content">
          <div class="stat-label">Healthy Backends</div>
          <div class="stat-value">{{ healthyBackends }}/{{ totalBackends }}</div>
          <div class="stat-trend" :class="{ positive: healthyBackends === totalBackends, negative: healthyBackends < totalBackends }">
            {{ healthyBackends === totalBackends ? '↗' : '↘' }} {{ healthyBackends === totalBackends ? 'All Online' : 'Some Down' }}
          </div>
        </div>
      </div>
    </div>

    <div class="charts-grid">
      <div class="chart-card glass fade-in" style="animation-delay: 0.4s">
        <h3>Request Distribution</h3>
        <div class="chart-container">
          <canvas ref="requestChart"></canvas>
        </div>
      </div>

      <div class="chart-card glass fade-in" style="animation-delay: 0.5s">
        <h3>Backend Status</h3>
        <div class="backend-list">
          <div v-for="backend in backends" :key="backend.address" class="backend-item">
            <div class="backend-status" :class="getStatusClass(backend.status)"></div>
            <div class="backend-info">
              <div class="backend-address">{{ backend.address }}</div>
              <div class="backend-pool">{{ backend.pool }}</div>
            </div>
            <div class="backend-status-text" :class="getStatusClass(backend.status)">
              {{ backend.status }}
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { ref, computed, watch, onMounted } from 'vue'
import { Chart, registerables } from 'chart.js'

Chart.register(...registerables)

export default {
  name: 'OverviewTab',
  props: {
    metrics: {
      type: Object,
      required: true
    },
    backends: {
      type: Array,
      required: true
    }
  },
  setup(props) {
    const requestChart = ref(null)
    let chartInstance = null

    const totalRequests = computed(() => {
      return Object.keys(props.metrics)
        .filter(k => k.includes('requests_total'))
        .reduce((sum, key) => sum + (props.metrics[key] || 0), 0)
    })

    const requestsPerSecond = computed(() => {
      const rps = Object.keys(props.metrics)
        .filter(k => k.includes('requests_per_second'))
        .reduce((sum, key) => sum + (props.metrics[key] || 0), 0)
      return rps || 0
    })

    const avgResponseTime = computed(() => {
      const times = Object.keys(props.metrics)
        .filter(k => k.includes('response_time_ms'))
        .map(key => props.metrics[key] || 0)

      if (times.length === 0) return 0
      return Math.round(times.reduce((a, b) => a + b, 0) / times.length)
    })

    const successRate = computed(() => {
      const total = totalRequests.value
      const errors = Object.keys(props.metrics)
        .filter(k => k.includes('errors_total'))
        .reduce((sum, key) => sum + (props.metrics[key] || 0), 0)

      if (total === 0) return 100
      return ((total - errors) / total * 100).toFixed(1)
    })

    const healthyBackends = computed(() => {
      return props.backends.filter(b => b.status === 'healthy' || b.status === 'Healthy').length
    })

    const totalBackends = computed(() => {
      return props.backends.length
    })

    const formatNumber = (num) => {
      if (num >= 1000000) return (num / 1000000).toFixed(1) + 'M'
      if (num >= 1000) return (num / 1000).toFixed(1) + 'K'
      return num.toString()
    }

    const getStatusClass = (status) => {
      const s = status.toLowerCase()
      if (s === 'healthy') return 'status-healthy'
      if (s === 'slow') return 'status-slow'
      return 'status-unhealthy'
    }

    const updateChart = () => {
      if (!requestChart.value) return

      const statusCodes = {}
      Object.keys(props.metrics).forEach(key => {
        if (key.includes('status_code=')) {
          const match = key.match(/status_code="(\d+)"/)
          if (match) {
            const code = match[1]
            statusCodes[code] = (statusCodes[code] || 0) + props.metrics[key]
          }
        }
      })

      const labels = Object.keys(statusCodes)
      const data = Object.values(statusCodes)

      if (chartInstance) {
        chartInstance.data.labels = labels
        chartInstance.data.datasets[0].data = data
        chartInstance.update()
      } else {
        chartInstance = new Chart(requestChart.value, {
          type: 'doughnut',
          data: {
            labels: labels.length > 0 ? labels : ['2xx', '3xx', '4xx', '5xx'],
            datasets: [{
              data: data.length > 0 ? data : [100, 20, 10, 5],
              backgroundColor: [
                'rgba(67, 233, 123, 0.8)',
                'rgba(79, 172, 254, 0.8)',
                'rgba(240, 147, 251, 0.8)',
                'rgba(248, 81, 73, 0.8)'
              ],
              borderColor: [
                'rgba(67, 233, 123, 1)',
                'rgba(79, 172, 254, 1)',
                'rgba(240, 147, 251, 1)',
                'rgba(248, 81, 73, 1)'
              ],
              borderWidth: 2
            }]
          },
          options: {
            responsive: true,
            maintainAspectRatio: false,
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
            }
          }
        })
      }
    }

    watch(() => props.metrics, updateChart, { deep: true })
    onMounted(updateChart)

    return {
      requestChart,
      totalRequests,
      requestsPerSecond,
      avgResponseTime,
      successRate,
      healthyBackends,
      totalBackends,
      formatNumber,
      getStatusClass
    }
  }
}
</script>

<style scoped>
.overview {
  display: flex;
  flex-direction: column;
  gap: 2rem;
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: 1.5rem;
}

.stat-card {
  padding: 1.5rem;
  border-radius: 12px;
  display: flex;
  gap: 1rem;
  transition: transform 0.2s;
}

.stat-card:hover {
  transform: translateY(-4px);
}

.stat-icon {
  width: 56px;
  height: 56px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 24px;
  flex-shrink: 0;
}

.stat-content {
  flex: 1;
}

.stat-label {
  font-size: 12px;
  color: #7d8590;
  margin-bottom: 0.25rem;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.stat-value {
  font-size: 28px;
  font-weight: 700;
  color: #e6edf3;
  margin-bottom: 0.25rem;
}

.stat-trend {
  font-size: 12px;
  font-weight: 500;
}

.stat-trend.positive {
  color: #3fb950;
}

.stat-trend.negative {
  color: #f85149;
}

.charts-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
  gap: 1.5rem;
}

.chart-card {
  padding: 1.5rem;
  border-radius: 12px;
}

.chart-card h3 {
  font-size: 18px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 1.5rem;
}

.chart-container {
  height: 300px;
}

.backend-list {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.backend-item {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1rem;
  background: rgba(255, 255, 255, 0.02);
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.05);
}

.backend-status {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  flex-shrink: 0;
}

.backend-status.status-healthy {
  background: #3fb950;
  box-shadow: 0 0 8px #3fb950;
}

.backend-status.status-slow {
  background: #d29922;
  box-shadow: 0 0 8px #d29922;
}

.backend-status.status-unhealthy {
  background: #f85149;
  box-shadow: 0 0 8px #f85149;
}

.backend-info {
  flex: 1;
}

.backend-address {
  font-size: 14px;
  font-weight: 500;
  color: #e6edf3;
}

.backend-pool {
  font-size: 12px;
  color: #7d8590;
  margin-top: 0.25rem;
}

.backend-status-text {
  font-size: 12px;
  font-weight: 500;
  padding: 0.25rem 0.75rem;
  border-radius: 6px;
  text-transform: uppercase;
}

.backend-status-text.status-healthy {
  background: rgba(63, 185, 80, 0.1);
  color: #3fb950;
}

.backend-status-text.status-slow {
  background: rgba(210, 153, 34, 0.1);
  color: #d29922;
}

.backend-status-text.status-unhealthy {
  background: rgba(248, 81, 73, 0.1);
  color: #f85149;
}
</style>
