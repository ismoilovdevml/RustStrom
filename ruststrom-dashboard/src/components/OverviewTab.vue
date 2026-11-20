<template>
  <div class="overview">
    <!-- Health Score Card -->
    <div class="health-score-card glass fade-in">
      <div class="health-score-content">
        <div class="score-circle" :class="healthScoreClass">
          <svg viewBox="0 0 100 100">
            <circle cx="50" cy="50" r="45" class="score-bg"></circle>
            <circle cx="50" cy="50" r="45" class="score-progress"
                    :style="{ strokeDashoffset: scoreOffset }"></circle>
          </svg>
          <div class="score-value">{{ healthScore }}</div>
        </div>
        <div class="score-info">
          <h3>System Health Score</h3>
          <p class="score-status" :class="healthScoreClass">{{ healthScoreStatus }}</p>
          <div class="score-factors">
            <span class="factor" :class="{ good: successRate >= 99 }">
              {{ successRate >= 99 ? '✓' : '✗' }} Success Rate
            </span>
            <span class="factor" :class="{ good: avgResponseTime < 100 }">
              {{ avgResponseTime < 100 ? '✓' : '✗' }} Response Time
            </span>
            <span class="factor" :class="{ good: healthyBackends === totalBackends }">
              {{ healthyBackends === totalBackends ? '✓' : '✗' }} Backends
            </span>
          </div>
        </div>
      </div>
    </div>

    <!-- Stats Grid -->
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

    <div class="content-grid">
      <!-- Live Activity Feed -->
      <div class="activity-card glass fade-in" style="animation-delay: 0.4s">
        <div class="activity-header">
          <h3>🔴 Live Activity</h3>
          <button class="btn-small" @click="toggleActivityFeed" :class="{ active: activityPaused }">
            {{ activityPaused ? '▶️ Resume' : '⏸️ Pause' }}
          </button>
        </div>
        <div class="activity-feed" ref="activityFeed">
          <div v-for="(activity, index) in recentActivities" :key="index" class="activity-item" :class="activity.type">
            <div class="activity-time">{{ activity.time }}</div>
            <div class="activity-content">
              <span class="activity-method" :class="activity.method">{{ activity.method }}</span>
              <span class="activity-path">{{ activity.path }}</span>
              <span class="activity-status" :class="activity.statusClass">{{ activity.status }}</span>
              <span class="activity-duration">{{ activity.duration }}ms</span>
            </div>
          </div>
        </div>
      </div>

      <!-- Quick Actions -->
      <div class="actions-card glass fade-in" style="animation-delay: 0.5s">
        <h3>⚡ Quick Actions</h3>
        <div class="actions-grid">
          <button class="action-btn" @click="reloadConfig">
            <span class="action-icon">🔄</span>
            <span class="action-label">Reload Config</span>
          </button>
          <button class="action-btn" @click="exportMetrics">
            <span class="action-icon">📥</span>
            <span class="action-label">Export Metrics</span>
          </button>
          <button class="action-btn" @click="exportLogs">
            <span class="action-icon">📋</span>
            <span class="action-label">Export Logs</span>
          </button>
          <button class="action-btn" @click="clearCache">
            <span class="action-icon">🗑️</span>
            <span class="action-label">Clear Cache</span>
          </button>
        </div>
      </div>
    </div>

    <!-- Charts Grid -->
    <div class="charts-grid">
      <div class="chart-card glass fade-in" style="animation-delay: 0.6s">
        <h3>📊 Request Distribution</h3>
        <div class="chart-container">
          <canvas ref="requestChart"></canvas>
        </div>
      </div>

      <div class="chart-card glass fade-in" style="animation-delay: 0.7s">
        <h3>🖥️ Backend Status</h3>
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
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { Chart, registerables } from 'chart.js'
import axios from 'axios'

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
  setup(props, { emit }) {
    const requestChart = ref(null)
    const activityFeed = ref(null)
    const activityPaused = ref(false)
    const recentActivities = ref([])
    let chartInstance = null
    let activityInterval = null

    // Health Score Calculation
    const healthScore = computed(() => {
      let score = 100

      // Success rate impact (50 points)
      const successRate = parseFloat(props.metrics.success_rate || 100)
      score -= (100 - successRate) * 0.5

      // Response time impact (30 points)
      const avgTime = parseFloat(props.metrics.avg_response_time_ms || 0)
      if (avgTime > 100) score -= Math.min(30, (avgTime - 100) / 10)

      // Backend health impact (20 points)
      const healthyBackends = props.backends.filter(b => b.status === 'healthy' || b.status === 'Healthy').length
      const totalBackends = props.backends.length
      if (totalBackends > 0) {
        score -= 20 * (1 - healthyBackends / totalBackends)
      }

      return Math.max(0, Math.round(score))
    })

    const healthScoreClass = computed(() => {
      const score = healthScore.value
      if (score >= 90) return 'excellent'
      if (score >= 70) return 'good'
      if (score >= 50) return 'warning'
      return 'critical'
    })

    const healthScoreStatus = computed(() => {
      const score = healthScore.value
      if (score >= 90) return '🎉 Excellent Performance'
      if (score >= 70) return '👍 Good Performance'
      if (score >= 50) return '⚠️ Performance Issues'
      return '🚨 Critical Issues'
    })

    const scoreOffset = computed(() => {
      const circumference = 2 * Math.PI * 45
      return circumference - (healthScore.value / 100) * circumference
    })

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
      if (props.metrics['avg_response_time_ms']) {
        return Math.round(props.metrics['avg_response_time_ms'])
      }
      return 0
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

    // Live Activity Feed
    const generateMockActivity = () => {
      if (activityPaused.value) return

      const methods = ['GET', 'POST', 'PUT', 'DELETE']
      const paths = ['/', '/api/users', '/api/products', '/health', '/metrics']
      const statuses = [200, 200, 200, 200, 201, 304, 400, 404, 500]

      const method = methods[Math.floor(Math.random() * methods.length)]
      const path = paths[Math.floor(Math.random() * paths.length)]
      const status = statuses[Math.floor(Math.random() * statuses.length)]
      const duration = Math.floor(Math.random() * 200) + 10

      const now = new Date()
      const time = now.toLocaleTimeString('en-US', { hour12: false })

      let statusClass = 'success'
      if (status >= 300 && status < 400) statusClass = 'redirect'
      if (status >= 400 && status < 500) statusClass = 'client-error'
      if (status >= 500) statusClass = 'server-error'

      const activity = {
        time,
        method,
        path,
        status,
        statusClass,
        duration,
        type: statusClass
      }

      recentActivities.value.unshift(activity)
      if (recentActivities.value.length > 20) {
        recentActivities.value.pop()
      }
    }

    const toggleActivityFeed = () => {
      activityPaused.value = !activityPaused.value
    }

    // Quick Actions
    const reloadConfig = async () => {
      try {
        await axios.post('/api/reload')
        alert('✅ Configuration reloaded successfully!')
      } catch (error) {
        alert('❌ Failed to reload configuration: ' + error.message)
      }
    }

    const exportMetrics = async () => {
      try {
        const response = await axios.get('/api/metrics')
        const blob = new Blob([response.data], { type: 'text/plain' })
        const url = URL.createObjectURL(blob)
        const link = document.createElement('a')
        link.href = url
        link.download = `metrics-${Date.now()}.txt`
        document.body.appendChild(link)
        link.click()
        document.body.removeChild(link)
        URL.revokeObjectURL(url)
      } catch (error) {
        alert('❌ Failed to export metrics: ' + error.message)
      }
    }

    const exportLogs = async () => {
      try {
        const response = await axios.get('/api/logs')
        const blob = new Blob([response.data], { type: 'text/plain' })
        const url = URL.createObjectURL(blob)
        const link = document.createElement('a')
        link.href = url
        link.download = `logs-${Date.now()}.txt`
        document.body.appendChild(link)
        link.click()
        document.body.removeChild(link)
        URL.revokeObjectURL(url)
      } catch (error) {
        alert('❌ Failed to export logs: ' + error.message)
      }
    }

    const clearCache = () => {
      localStorage.removeItem('metricsHistory')
      alert('✅ Cache cleared successfully!')
      window.location.reload()
    }

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
        if (key.includes('http_status_codes_total{code=')) {
          const match = key.match(/code="(\d+)"/)
          if (match) {
            const code = match[1]
            statusCodes[code] = (statusCodes[code] || 0) + props.metrics[key]
          }
        }
      })

      const labels = Object.keys(statusCodes).sort()
      const data = labels.map(code => statusCodes[code])

      if (chartInstance) {
        chartInstance.data.labels = labels.length > 0 ? labels : ['200', '404']
        chartInstance.data.datasets[0].data = data.length > 0 ? data : [100, 5]
        chartInstance.update()
      } else {
        chartInstance = new Chart(requestChart.value, {
          type: 'doughnut',
          data: {
            labels: labels.length > 0 ? labels : ['200', '404'],
            datasets: [{
              data: data.length > 0 ? data : [100, 5],
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
    onMounted(() => {
      updateChart()
      activityInterval = setInterval(generateMockActivity, 1000)
    })

    onUnmounted(() => {
      if (chartInstance) {
        chartInstance.destroy()
      }
      if (activityInterval) {
        clearInterval(activityInterval)
      }
    })

    return {
      requestChart,
      activityFeed,
      activityPaused,
      recentActivities,
      healthScore,
      healthScoreClass,
      healthScoreStatus,
      scoreOffset,
      totalRequests,
      requestsPerSecond,
      avgResponseTime,
      successRate,
      healthyBackends,
      totalBackends,
      formatNumber,
      getStatusClass,
      toggleActivityFeed,
      reloadConfig,
      exportMetrics,
      exportLogs,
      clearCache
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

/* Health Score Card */
.health-score-card {
  padding: 2rem;
  border-radius: 12px;
}

.health-score-content {
  display: flex;
  gap: 2rem;
  align-items: center;
}

.score-circle {
  position: relative;
  width: 120px;
  height: 120px;
  flex-shrink: 0;
}

.score-circle svg {
  transform: rotate(-90deg);
  width: 100%;
  height: 100%;
}

.score-bg {
  fill: none;
  stroke: rgba(255, 255, 255, 0.1);
  stroke-width: 8;
}

.score-progress {
  fill: none;
  stroke-width: 8;
  stroke-linecap: round;
  stroke-dasharray: 283;
  transition: stroke-dashoffset 0.5s ease;
}

.score-circle.excellent .score-progress {
  stroke: #3fb950;
}

.score-circle.good .score-progress {
  stroke: #4facfe;
}

.score-circle.warning .score-progress {
  stroke: #f093fb;
}

.score-circle.critical .score-progress {
  stroke: #f85149;
}

.score-value {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  font-size: 32px;
  font-weight: 700;
  color: #e6edf3;
}

.score-info {
  flex: 1;
}

.score-info h3 {
  font-size: 24px;
  font-weight: 700;
  color: #e6edf3;
  margin-bottom: 0.5rem;
}

.score-status {
  font-size: 16px;
  font-weight: 600;
  margin-bottom: 1rem;
}

.score-status.excellent {
  color: #3fb950;
}

.score-status.good {
  color: #4facfe;
}

.score-status.warning {
  color: #f093fb;
}

.score-status.critical {
  color: #f85149;
}

.score-factors {
  display: flex;
  gap: 1rem;
  flex-wrap: wrap;
}

.factor {
  padding: 0.5rem 1rem;
  background: rgba(248, 81, 73, 0.1);
  color: #f85149;
  border-radius: 6px;
  font-size: 13px;
  font-weight: 500;
}

.factor.good {
  background: rgba(63, 185, 80, 0.1);
  color: #3fb950;
}

/* Stats Grid */
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

/* Content Grid */
.content-grid {
  display: grid;
  grid-template-columns: 2fr 1fr;
  gap: 1.5rem;
}

@media (max-width: 1200px) {
  .content-grid {
    grid-template-columns: 1fr;
  }
}

/* Activity Feed */
.activity-card {
  padding: 1.5rem;
  border-radius: 12px;
}

.activity-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

.activity-header h3 {
  font-size: 18px;
  font-weight: 600;
  color: #e6edf3;
}

.activity-feed {
  max-height: 400px;
  overflow-y: auto;
}

.activity-feed::-webkit-scrollbar {
  width: 6px;
}

.activity-feed::-webkit-scrollbar-track {
  background: rgba(255, 255, 255, 0.05);
  border-radius: 3px;
}

.activity-feed::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.2);
  border-radius: 3px;
}

.activity-item {
  display: flex;
  gap: 1rem;
  padding: 0.75rem;
  margin-bottom: 0.5rem;
  background: rgba(255, 255, 255, 0.02);
  border-radius: 6px;
  border-left: 3px solid;
  animation: slideIn 0.3s ease;
}

@keyframes slideIn {
  from {
    opacity: 0;
    transform: translateX(-10px);
  }
  to {
    opacity: 1;
    transform: translateX(0);
  }
}

.activity-item.success {
  border-left-color: #3fb950;
}

.activity-item.redirect {
  border-left-color: #4facfe;
}

.activity-item.client-error {
  border-left-color: #f093fb;
}

.activity-item.server-error {
  border-left-color: #f85149;
}

.activity-time {
  font-size: 11px;
  color: #7d8590;
  font-family: 'Courier New', monospace;
  min-width: 60px;
}

.activity-content {
  display: flex;
  gap: 0.75rem;
  align-items: center;
  flex: 1;
  font-size: 12px;
}

.activity-method {
  font-weight: 600;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
  font-size: 11px;
}

.activity-method.GET {
  background: rgba(67, 233, 123, 0.1);
  color: #3fb950;
}

.activity-method.POST {
  background: rgba(79, 172, 254, 0.1);
  color: #4facfe;
}

.activity-method.PUT {
  background: rgba(240, 147, 251, 0.1);
  color: #f093fb;
}

.activity-method.DELETE {
  background: rgba(248, 81, 73, 0.1);
  color: #f85149;
}

.activity-path {
  color: #e6edf3;
  font-family: 'Courier New', monospace;
  flex: 1;
}

.activity-status {
  font-weight: 600;
}

.activity-status.success {
  color: #3fb950;
}

.activity-status.redirect {
  color: #4facfe;
}

.activity-status.client-error {
  color: #f093fb;
}

.activity-status.server-error {
  color: #f85149;
}

.activity-duration {
  color: #7d8590;
}

/* Quick Actions */
.actions-card {
  padding: 1.5rem;
  border-radius: 12px;
}

.actions-card h3 {
  font-size: 18px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 1rem;
}

.actions-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1rem;
}

.action-btn {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  padding: 1.5rem;
  background: rgba(255, 255, 255, 0.02);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.2s;
  color: #e6edf3;
}

.action-btn:hover {
  background: rgba(255, 255, 255, 0.05);
  border-color: rgba(102, 126, 234, 0.5);
  transform: translateY(-2px);
}

.action-icon {
  font-size: 32px;
}

.action-label {
  font-size: 13px;
  font-weight: 500;
}

.btn-small {
  padding: 0.375rem 0.875rem;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 6px;
  background: transparent;
  color: #7d8590;
  font-size: 12px;
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

/* Charts Grid */
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
