<template>
  <div class="config">
    <div class="config-header glass fade-in">
      <div>
        <h3>Configuration Editor</h3>
        <p class="config-description">Edit and reload RustStrom configuration in real-time</p>
      </div>
      <div class="config-actions">
        <button class="btn btn-secondary" @click="resetConfig">
          <span>↺</span> Reset
        </button>
        <button class="btn btn-primary" @click="saveConfig">
          <span>💾</span> Save
        </button>
        <button class="btn btn-success" @click="reloadConfig">
          <span>🔄</span> Reload
        </button>
      </div>
    </div>

    <div class="config-editor glass fade-in" style="animation-delay: 0.1s">
      <div class="editor-toolbar">
        <div class="editor-info">
          <span class="info-item">
            <span class="info-label">Lines:</span>
            <span class="info-value">{{ lineCount }}</span>
          </span>
          <span class="info-item">
            <span class="info-label">Format:</span>
            <span class="info-value">TOML</span>
          </span>
          <span class="info-item">
            <span class="info-label">Status:</span>
            <span class="info-value" :class="{ 'status-modified': isModified, 'status-saved': !isModified }">
              {{ isModified ? 'Modified' : 'Saved' }}
            </span>
          </span>
        </div>
        <button class="btn-icon" @click="copyConfig" title="Copy to clipboard">
          📋
        </button>
      </div>
      <textarea
        v-model="editableConfig"
        class="config-textarea"
        spellcheck="false"
        @input="handleInput"
      ></textarea>
    </div>

    <div class="config-help glass fade-in" style="animation-delay: 0.2s">
      <h4>Quick Reference</h4>
      <div class="help-grid">
        <div class="help-item">
          <div class="help-icon">🎯</div>
          <div>
            <div class="help-title">Backend Pools</div>
            <div class="help-text">Configure backend servers with health checks and load balancing strategies</div>
          </div>
        </div>
        <div class="help-item">
          <div class="help-icon">⚡</div>
          <div>
            <div class="help-title">Strategies</div>
            <div class="help-text">RoundRobin, Random, IPHash, LeastConnection, StickyCookie</div>
          </div>
        </div>
        <div class="help-item">
          <div class="help-icon">🛡️</div>
          <div>
            <div class="help-title">Middlewares</div>
            <div class="help-text">Compression, RateLimiting, Authentication, HTTPS Redirect</div>
          </div>
        </div>
        <div class="help-item">
          <div class="help-icon">📊</div>
          <div>
            <div class="help-title">Metrics</div>
            <div class="help-text">Enable Prometheus metrics endpoint for monitoring</div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { ref, computed, watch } from 'vue'

export default {
  name: 'ConfigTab',
  props: {
    config: {
      type: String,
      required: true
    }
  },
  emits: ['update', 'reload'],
  setup(props, { emit }) {
    const editableConfig = ref(props.config)
    const originalConfig = ref(props.config)
    const isModified = ref(false)

    const lineCount = computed(() => {
      return editableConfig.value.split('\n').length
    })

    const handleInput = () => {
      isModified.value = editableConfig.value !== originalConfig.value
    }

    const saveConfig = () => {
      emit('update', editableConfig.value)
      originalConfig.value = editableConfig.value
      isModified.value = false
    }

    const resetConfig = () => {
      if (isModified.value) {
        if (confirm('Are you sure you want to discard your changes?')) {
          editableConfig.value = originalConfig.value
          isModified.value = false
        }
      }
    }

    const reloadConfig = () => {
      if (isModified.value) {
        if (confirm('You have unsaved changes. Save before reloading?')) {
          saveConfig()
        }
      }
      emit('reload')
    }

    const copyConfig = () => {
      navigator.clipboard.writeText(editableConfig.value)
      alert('Configuration copied to clipboard!')
    }

    watch(() => props.config, (newVal) => {
      if (!isModified.value) {
        editableConfig.value = newVal
        originalConfig.value = newVal
      }
    })

    return {
      editableConfig,
      isModified,
      lineCount,
      handleInput,
      saveConfig,
      resetConfig,
      reloadConfig,
      copyConfig
    }
  }
}
</script>

<style scoped>
.config {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.config-header {
  padding: 1.5rem;
  border-radius: 12px;
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.config-header h3 {
  font-size: 20px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 0.5rem;
}

.config-description {
  font-size: 14px;
  color: #7d8590;
  margin: 0;
}

.config-actions {
  display: flex;
  gap: 0.75rem;
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

.btn-secondary {
  background: rgba(255, 255, 255, 0.05);
  color: #e6edf3;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.btn-secondary:hover {
  background: rgba(255, 255, 255, 0.1);
}

.btn-success {
  background: linear-gradient(135deg, #43e97b 0%, #38f9d7 100%);
  color: #0f1419;
}

.btn-success:hover {
  transform: translateY(-2px);
  box-shadow: 0 4px 12px rgba(67, 233, 123, 0.4);
}

.config-editor {
  padding: 1.5rem;
  border-radius: 12px;
  display: flex;
  flex-direction: column;
}

.editor-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-bottom: 1rem;
  margin-bottom: 1rem;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}

.editor-info {
  display: flex;
  gap: 1.5rem;
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

.info-value.status-modified {
  color: #d29922;
}

.info-value.status-saved {
  color: #3fb950;
}

.btn-icon {
  padding: 0.5rem;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 6px;
  color: #e6edf3;
  cursor: pointer;
  transition: all 0.2s;
  font-size: 16px;
}

.btn-icon:hover {
  background: rgba(255, 255, 255, 0.1);
}

.config-textarea {
  width: 100%;
  min-height: 500px;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.05);
  border-radius: 8px;
  padding: 1rem;
  color: #e6edf3;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 13px;
  line-height: 1.6;
  resize: vertical;
  outline: none;
}

.config-textarea:focus {
  border-color: rgba(102, 126, 234, 0.5);
  box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.1);
}

.config-help {
  padding: 1.5rem;
  border-radius: 12px;
}

.config-help h4 {
  font-size: 16px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 1rem;
}

.help-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: 1rem;
}

.help-item {
  display: flex;
  gap: 1rem;
  padding: 1rem;
  background: rgba(255, 255, 255, 0.02);
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.05);
}

.help-icon {
  font-size: 24px;
  flex-shrink: 0;
}

.help-title {
  font-size: 14px;
  font-weight: 600;
  color: #e6edf3;
  margin-bottom: 0.25rem;
}

.help-text {
  font-size: 12px;
  color: #7d8590;
  line-height: 1.5;
}
</style>
