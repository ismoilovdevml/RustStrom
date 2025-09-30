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
      <div class="metric-card glass fade-in" style="animation-delay: 0.1s">
        <h4>Request Metrics</h4>
        <div class="metric-list">
          <div v-for="(value, key) in requestMetrics" :key="key" class="metric-item">
            <span class="metric-key">{{ formatKey(key) }}</span>
            <span class="metric-value">{{ formatValue(value) }}</span>
          </div>
        </div>
      </div>

      <div class="metric-card glass fade-in" style="animation-delay: 0.2s">
        <h4>Response Time Metrics</h4>
        <div class="metric-list">
          <div v-for="(value, key) in responseMetrics" :key="key" class="metric-item">
            <span class="metric-key">{{ formatKey(key) }}</span>
            <span class="metric-value">{{ value }}ms</span>
          </div>
        </div>
      </div>

      <div class="metric-card glass fade-in" style="animation-delay: 0.3s">
        <h4>Error Metrics</h4>
        <div class="metric-list">
          <div v-for="(value, key) in errorMetrics" :key="key" class="metric-item">
            <span class="metric-key">{{ formatKey(key) }}</span>
            <span class="metric-value error">{{ formatValue(value) }}</span>
          </div>
        </div>
      </div>

      <div class="metric-card glass fade-in" style="animation-delay: 0.4s">
        <h4>Backend Metrics</h4>
        <div class="metric-list">
          <div v-for="(value, key) in backendMetrics" :key="key" class="metric-item">
            <span class="metric-key">{{ formatKey(key) }}</span>
            <span class="metric-value">{{ formatValue(value) }}</span>
          </div>
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

    const requestMetrics = computed(() => {
      return Object.keys(props.metrics)
        .filter(k => k.includes('requests') && !k.includes('response_time'))
        .reduce((acc, key) => {
          acc[key] = props.metrics[key]
          return acc
        }, {})
    })

    const responseMetrics = computed(() => {
      return Object.keys(props.metrics)
        .filter(k => k.includes('response_time'))
        .reduce((acc, key) => {
          acc[key] = props.metrics[key]
          return acc
        }, {})
    })

    const errorMetrics = computed(() => {
      return Object.keys(props.metrics)
        .filter(k => k.includes('error') || k.includes('timeout'))
        .reduce((acc, key) => {
          acc[key] = props.metrics[key]
          return acc
        }, {})
    })

    const backendMetrics = computed(() => {
      return Object.keys(props.metrics)
        .filter(k => k.includes('backend') && !k.includes('response_time'))
        .reduce((acc, key) => {
          acc[key] = props.metrics[key]
          return acc
        }, {})
    })

    const formatKey = (key) => {
      return key
        .replace(/_/g, ' ')
        .replace(/\{[^}]+\}/g, '')
        .trim()
        .split(' ')
        .map(w => w.charAt(0).toUpperCase() + w.slice(1))
        .join(' ')
    }

    const formatValue = (value) => {
      if (typeof value !== 'number') return value
      if (value >= 1000000) return (value / 1000000).toFixed(2) + 'M'
      if (value >= 1000) return (value / 1000).toFixed(2) + 'K'
      return value.toFixed(2)
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

      const requestsData = filteredHistory.map(h => {
        return Object.keys(h)
          .filter(k => k.includes('requests_total'))
          .reduce((sum, key) => sum + (h[key] || 0), 0)
      })

      const responseTimeData = filteredHistory.map(h => {
        const times = Object.keys(h)
          .filter(k => k.includes('response_time_ms'))
          .map(key => h[key] || 0)
        return times.length > 0 ? times.reduce((a, b) => a + b, 0) / times.length : 0
      })

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
      formatKey,
      formatValue
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
  padding: 0.5rem;
  background: rgba(255, 255, 255, 0.02);
  border-radius: 6px;
}

.metric-key {
  font-size: 13px;
  color: #7d8590;
}

.metric-value {
  font-size: 14px;
  font-weight: 600;
  color: #e6edf3;
}

.metric-value.error {
  color: #f85149;
}
</style>
