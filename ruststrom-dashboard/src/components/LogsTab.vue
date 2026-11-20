<template>
  <div class="logs">
    <div class="logs-header glass fade-in">
      <div>
        <h3>System Logs</h3>
        <p class="logs-description">Real-time RustStrom load balancer logs from /tmp/ruststrom.log</p>
      </div>
      <div class="logs-actions">
        <select v-model="logLevel" class="log-level-select">
          <option value="all">All Levels</option>
          <option value="error">Error</option>
          <option value="warn">Warning</option>
          <option value="info">Info</option>
          <option value="debug">Debug</option>
        </select>
        <input 
          v-model="searchQuery" 
          type="text" 
          class="search-input" 
          placeholder="🔍 Search logs..."
        />
        <button class="btn btn-secondary" @click="copyLogs">
          <span>📋</span> Copy
        </button>
        <button class="btn btn-secondary" @click="downloadLogs">
          <span>💾</span> Download
        </button>
        <button class="btn btn-secondary" @click="clearLogs">
          <span>🗑️</span> Clear
        </button>
        <button class="btn btn-primary" @click="refreshLogs">
          <span>🔄</span> Refresh
        </button>
      </div>
    </div>

    <div class="logs-viewer glass fade-in" style="animation-delay: 0.1s">
      <div class="logs-toolbar">
        <div class="logs-info">
          <span class="info-item">
            <span class="info-label">Total:</span>
            <span class="info-value">{{ filteredLogs.length }} / {{ logs.length }} logs</span>
          </span>
          <span class="info-item">
            <span class="info-label">Last Updated:</span>
            <span class="info-value">{{ lastUpdated }}</span>
          </span>
          <span class="info-item">
            <span class="info-label">Auto-refresh:</span>
            <span class="info-value" :class="{ 'status-online': autoRefresh, 'status-offline': !autoRefresh }">
              {{ autoRefresh ? 'ON' : 'OFF' }}
            </span>
          </span>
        </div>
        <div class="toolbar-actions">
          <button class="btn-toggle" @click="autoRefresh = !autoRefresh">
            {{ autoRefresh ? '⏸️ Pause' : '▶️ Resume' }}
          </button>
          <button class="btn-toggle" @click="autoScroll = !autoScroll">
            {{ autoScroll ? '📌 Pin' : '📍 Auto-scroll' }}
          </button>
        </div>
      </div>
      <div class="logs-container" ref="logsContainer">
        <div 
          v-for="(log, index) in filteredLogs" 
          :key="index" 
          class="log-entry"
          :class="`log-${log.level}`"
        >
          <span class="log-time" :title="log.fullTimestamp">{{ log.timestamp }}</span>
          <span class="log-level">{{ log.level.toUpperCase() }}</span>
          <span class="log-message">{{ log.message }}</span>
        </div>
        <div v-if="filteredLogs.length === 0" class="no-logs">
          <div class="no-logs-icon">📝</div>
          <div class="no-logs-text">{{ logs.length === 0 ? 'No logs available' : 'No matching logs' }}</div>
          <div class="no-logs-hint">
            {{ logs.length === 0 
              ? 'Reading from /tmp/ruststrom.log - logs will appear when RustStrom generates them' 
              : 'Try changing your filter or search query' }}
          </div>
        </div>
      </div>
    </div>

    <div class="logs-stats glass fade-in" style="animation-delay: 0.2s">
      <h4>Log Statistics</h4>
      <div class="stats-grid">
        <div class="stat-item stat-error" @click="logLevel = 'error'">
          <div class="stat-icon">🔴</div>
          <div>
            <div class="stat-value">{{ logStats.error }}</div>
            <div class="stat-label">Errors</div>
          </div>
        </div>
        <div class="stat-item stat-warn" @click="logLevel = 'warn'">
          <div class="stat-icon">🟡</div>
          <div>
            <div class="stat-value">{{ logStats.warn }}</div>
            <div class="stat-label">Warnings</div>
          </div>
        </div>
        <div class="stat-item stat-info" @click="logLevel = 'info'">
          <div class="stat-icon">🔵</div>
          <div>
            <div class="stat-value">{{ logStats.info }}</div>
            <div class="stat-label">Info</div>
          </div>
        </div>
        <div class="stat-item stat-debug" @click="logLevel = 'debug'">
          <div class="stat-icon">⚪</div>
          <div>
            <div class="stat-value">{{ logStats.debug }}</div>
            <div class="stat-label">Debug</div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import axios from 'axios'

export default {
  name: 'LogsTab',
  setup() {
    const logs = ref([])
    const logLevel = ref('all')
    const searchQuery = ref('')
    const autoRefresh = ref(true)
    const autoScroll = ref(true)
    const logsContainer = ref(null)
    const lastUpdated = ref('Never')
    let refreshInterval = null

    const filteredLogs = computed(() => {
      let filtered = logs.value
      
      // Filter by level
      if (logLevel.value !== 'all') {
        filtered = filtered.filter(log => log.level === logLevel.value)
      }
      
      // Filter by search query
      if (searchQuery.value.trim()) {
        const query = searchQuery.value.toLowerCase()
        filtered = filtered.filter(log => 
          log.message.toLowerCase().includes(query) ||
          log.level.toLowerCase().includes(query) ||
          log.timestamp.toLowerCase().includes(query)
        )
      }
      
      return filtered
    })

    const logStats = computed(() => {
      return {
        error: logs.value.filter(l => l.level === 'error').length,
        warn: logs.value.filter(l => l.level === 'warn').length,
        info: logs.value.filter(l => l.level === 'info').length,
        debug: logs.value.filter(l => l.level === 'debug').length
      }
    })

    const parseLogLine = (line) => {
      // Parse log format from log4rs or standard Rust logs
      // Example: "2025-01-30 10:57:39 WARN Backend health check failed for 127.0.0.1:8080"
      const patterns = [
        /^(\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2})\s+(ERROR|WARN|INFO|DEBUG)\s+(.+)$/i,
        /^\[(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})\]\s+(ERROR|WARN|INFO|DEBUG)\s+(.+)$/i,
        /^(ERROR|WARN|INFO|DEBUG)\s+(.+)$/i
      ]
      
      for (const pattern of patterns) {
        const match = line.match(pattern)
        if (match) {
          if (pattern.source.includes('\\d{4}')) {
            // Has timestamp
            const timestamp = match[1]
            const level = match[2].toLowerCase()
            const message = match[3]
            return {
              fullTimestamp: timestamp,
              timestamp: formatTimestamp(timestamp),
              level: level,
              message: message
            }
          } else {
            // No timestamp, use current time
            const now = new Date()
            return {
              fullTimestamp: now.toISOString(),
              timestamp: formatTimestamp(now.toISOString()),
              level: match[1].toLowerCase(),
              message: match[2]
            }
          }
        }
      }
      
      // If no pattern matches, treat as info
      const now = new Date()
      return {
        fullTimestamp: now.toISOString(),
        timestamp: formatTimestamp(now.toISOString()),
        level: 'info',
        message: line
      }
    }

    const formatTimestamp = (ts) => {
      try {
        const date = new Date(ts)
        return date.toLocaleTimeString('en-US', { hour12: false })
      } catch (e) {
        return ts
      }
    }

    const refreshLogs = async () => {
      try {
        const response = await axios.get('/api/logs')
        
        if (response.data && typeof response.data === 'string') {
          // Raw log file content
          const lines = response.data.split('\n').filter(line => line.trim())
          logs.value = lines.map(parseLogLine)
        } else if (response.data && response.data.logs) {
          // JSON format
          logs.value = response.data.logs
        } else {
          console.warn('Unexpected log format, using empty logs')
          logs.value = []
        }
        
        lastUpdated.value = new Date().toLocaleTimeString('en-US', { hour12: false })
        
        // Auto-scroll to bottom if enabled
        if (autoScroll.value && logsContainer.value) {
          setTimeout(() => {
            logsContainer.value.scrollTop = logsContainer.value.scrollHeight
          }, 50)
        }
      } catch (error) {
        console.error('Failed to fetch logs:', error)
        // Don't clear logs on error, keep last known state
      }
    }

    const copyLogs = () => {
      const logsText = filteredLogs.value
        .map(log => `${log.fullTimestamp || log.timestamp} ${log.level.toUpperCase()} ${log.message}`)
        .join('\n')
      
      navigator.clipboard.writeText(logsText)
        .then(() => alert(`Copied ${filteredLogs.value.length} log entries to clipboard!`))
        .catch(() => alert('Failed to copy logs'))
    }

    const downloadLogs = () => {
      const logsText = filteredLogs.value
        .map(log => `${log.fullTimestamp || log.timestamp} ${log.level.toUpperCase()} ${log.message}`)
        .join('\n')
      
      const blob = new Blob([logsText], { type: 'text/plain' })
      const url = URL.createObjectURL(blob)
      const link = document.createElement('a')
      link.href = url
      link.download = `ruststrom-logs-${Date.now()}.txt`
      document.body.appendChild(link)
      link.click()
      document.body.removeChild(link)
      URL.revokeObjectURL(url)
      
      alert(`Downloaded ${filteredLogs.value.length} log entries`)
    }

    const clearLogs = () => {
      if (confirm('Are you sure you want to clear all logs from view?')) {
        logs.value = []
        lastUpdated.value = 'Cleared'
      }
    }

    watch(autoRefresh, (newVal) => {
      if (newVal) {
        refreshInterval = setInterval(refreshLogs, 3000)
      } else {
        if (refreshInterval) {
          clearInterval(refreshInterval)
        }
      }
    })

    onMounted(() => {
      refreshLogs()
      if (autoRefresh.value) {
        refreshInterval = setInterval(refreshLogs, 3000)
      }
    })

    onUnmounted(() => {
      if (refreshInterval) {
        clearInterval(refreshInterval)
      }
    })

    return {
      logs,
      logLevel,
      searchQuery,
      autoRefresh,
      autoScroll,
      logsContainer,
      lastUpdated,
      filteredLogs,
      logStats,
      refreshLogs,
      copyLogs,
      downloadLogs,
      clearLogs
    }
  }
}
</script>

<style scoped>
.logs {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.logs-header {
  padding: 1.5rem;
  border-radius: 12px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 1rem;
}

.logs-header h3 {
  font-size: 20px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 0.5rem;
}

.logs-description {
  font-size: 14px;
  color: #7d8590;
  margin: 0;
}

.logs-actions {
  display: flex;
  gap: 0.75rem;
  align-items: center;
  flex-wrap: wrap;
}

.log-level-select,
.search-input {
  padding: 0.625rem 1rem;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 8px;
  color: #e6edf3;
  font-size: 14px;
  outline: none;
}

.search-input {
  min-width: 200px;
}

.search-input::placeholder {
  color: #7d8590;
}

.log-level-select:hover,
.search-input:focus {
  background: rgba(255, 255, 255, 0.1);
  border-color: rgba(102, 126, 234, 0.5);
}

.log-level-select {
  cursor: pointer;
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
  white-space: nowrap;
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

.logs-viewer {
  padding: 1.5rem;
  border-radius: 12px;
  display: flex;
  flex-direction: column;
}

.logs-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-bottom: 1rem;
  margin-bottom: 1rem;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  flex-wrap: wrap;
  gap: 1rem;
}

.logs-info {
  display: flex;
  gap: 1.5rem;
  flex-wrap: wrap;
}

.toolbar-actions {
  display: flex;
  gap: 0.5rem;
}

.info-item {
  display: flex;
  gap: 0.5rem;
  font-size: 13px;
}

.info-label {
  color: #7d8590;
}

.info-value {
  color: #e6edf3;
  font-weight: 500;
}

.status-online {
  color: #3fb950;
}

.status-offline {
  color: #f85149;
}

.btn-toggle {
  padding: 0.5rem 1rem;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 6px;
  color: #e6edf3;
  cursor: pointer;
  transition: all 0.2s;
  font-size: 13px;
  white-space: nowrap;
}

.btn-toggle:hover {
  background: rgba(255, 255, 255, 0.1);
}

.logs-container {
  max-height: 500px;
  overflow-y: auto;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.05);
  border-radius: 8px;
  padding: 1rem;
}

.logs-container::-webkit-scrollbar {
  width: 8px;
}

.logs-container::-webkit-scrollbar-track {
  background: rgba(0, 0, 0, 0.2);
}

.logs-container::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.1);
  border-radius: 4px;
}

.logs-container::-webkit-scrollbar-thumb:hover {
  background: rgba(255, 255, 255, 0.2);
}

.log-entry {
  display: flex;
  gap: 1rem;
  padding: 0.75rem;
  border-radius: 6px;
  margin-bottom: 0.5rem;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 13px;
  line-height: 1.5;
  border-left: 3px solid transparent;
}

.log-entry:hover {
  background: rgba(255, 255, 255, 0.02);
}

.log-time {
  color: #7d8590;
  min-width: 80px;
  cursor: help;
}

.log-level {
  font-weight: 600;
  min-width: 60px;
}

.log-message {
  color: #e6edf3;
  flex: 1;
  word-break: break-word;
}

.log-error {
  border-left-color: #f85149;
  background: rgba(248, 81, 73, 0.03);
}

.log-error .log-level {
  color: #f85149;
}

.log-warn {
  border-left-color: #d29922;
  background: rgba(210, 153, 34, 0.03);
}

.log-warn .log-level {
  color: #d29922;
}

.log-info {
  border-left-color: #58a6ff;
  background: rgba(88, 166, 255, 0.03);
}

.log-info .log-level {
  color: #58a6ff;
}

.log-debug {
  border-left-color: #7d8590;
  background: rgba(125, 133, 144, 0.03);
}

.log-debug .log-level {
  color: #7d8590;
}

.no-logs {
  text-align: center;
  padding: 3rem 1rem;
}

.no-logs-icon {
  font-size: 48px;
  margin-bottom: 1rem;
}

.no-logs-text {
  font-size: 16px;
  font-weight: 500;
  color: #e6edf3;
  margin-bottom: 0.5rem;
}

.no-logs-hint {
  font-size: 14px;
  color: #7d8590;
  max-width: 500px;
  margin: 0 auto;
}

.logs-stats {
  padding: 1.5rem;
  border-radius: 12px;
}

.logs-stats h4 {
  font-size: 16px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 1rem;
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 1rem;
}

.stat-item {
  display: flex;
  gap: 1rem;
  padding: 1rem;
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.05);
  cursor: pointer;
  transition: all 0.2s;
}

.stat-item:hover {
  transform: translateY(-2px);
  border-color: rgba(255, 255, 255, 0.15);
}

.stat-icon {
  font-size: 24px;
  flex-shrink: 0;
}

.stat-value {
  font-size: 24px;
  font-weight: 700;
  color: #e6edf3;
  margin-bottom: 0.25rem;
}

.stat-label {
  font-size: 12px;
  color: #7d8590;
}

.stat-error {
  background: rgba(248, 81, 73, 0.05);
}

.stat-warn {
  background: rgba(210, 153, 34, 0.05);
}

.stat-info {
  background: rgba(88, 166, 255, 0.05);
}

.stat-debug {
  background: rgba(125, 133, 144, 0.05);
}
</style>
