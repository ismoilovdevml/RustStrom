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
      <div class="editor-container">
        <div class="line-numbers">
          <div v-for="n in lineCount" :key="n" class="line-number">{{ n }}</div>
        </div>
        <div class="editor-wrapper">
          <pre class="syntax-highlight"><code v-html="highlightedCode"></code></pre>
          <textarea
            v-model="editableConfig"
            class="config-textarea"
            spellcheck="false"
            @input="handleInput"
            @scroll="syncScroll"
            ref="textareaRef"
          ></textarea>
        </div>
      </div>
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
import Prism from 'prismjs'
import 'prismjs/components/prism-toml'
import 'prismjs/themes/prism-tomorrow.css'

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
    const textareaRef = ref(null)

    const lineCount = computed(() => {
      return editableConfig.value.split('\n').length
    })

    const highlightedCode = computed(() => {
      return Prism.highlight(editableConfig.value, Prism.languages.toml, 'toml')
    })

    const handleInput = () => {
      isModified.value = editableConfig.value !== originalConfig.value
    }

    const syncScroll = (event) => {
      const highlights = event.target.previousElementSibling
      if (highlights) {
        highlights.scrollTop = event.target.scrollTop
        highlights.scrollLeft = event.target.scrollLeft
      }
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
      highlightedCode,
      textareaRef,
      handleInput,
      syncScroll,
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

.editor-container {
  display: flex;
  gap: 0;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.05);
  border-radius: 8px;
  overflow: hidden;
}

.line-numbers {
  display: flex;
  flex-direction: column;
  padding: 1rem 0;
  background: rgba(0, 0, 0, 0.2);
  border-right: 1px solid rgba(255, 255, 255, 0.05);
  user-select: none;
}

.line-number {
  padding: 0 1rem;
  color: #7d8590;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 13px;
  line-height: 1.6;
  text-align: right;
  min-width: 40px;
}

.editor-wrapper {
  position: relative;
  flex: 1;
  overflow: hidden;
}

.syntax-highlight {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  padding: 1rem;
  margin: 0;
  background: transparent;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 13px;
  line-height: 1.6;
  overflow: auto;
  pointer-events: none;
  white-space: pre;
  word-wrap: normal;
}

.syntax-highlight code {
  display: block;
  background: transparent !important;
  padding: 0 !important;
  margin: 0 !important;
  font-family: inherit;
  font-size: inherit;
  line-height: inherit;
}

.config-textarea {
  position: relative;
  width: 100%;
  min-height: 500px;
  background: transparent;
  border: none;
  padding: 1rem;
  color: #e6edf3;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 13px;
  line-height: 1.6;
  resize: vertical;
  outline: none;
  caret-color: #e6edf3;
  overflow: auto;
  white-space: pre;
  word-wrap: normal;
}

.config-textarea::-webkit-scrollbar,
.syntax-highlight::-webkit-scrollbar {
  width: 8px;
  height: 8px;
}

.config-textarea::-webkit-scrollbar-track,
.syntax-highlight::-webkit-scrollbar-track {
  background: rgba(0, 0, 0, 0.2);
}

.config-textarea::-webkit-scrollbar-thumb,
.syntax-highlight::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.1);
  border-radius: 4px;
}

.config-textarea::-webkit-scrollbar-thumb:hover,
.syntax-highlight::-webkit-scrollbar-thumb:hover {
  background: rgba(255, 255, 255, 0.2);
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

/* Prism theme override */
:deep(.token.comment),
:deep(.token.prolog),
:deep(.token.doctype),
:deep(.token.cdata) {
  color: #7d8590;
}

:deep(.token.punctuation) {
  color: #c9d1d9;
}

:deep(.token.property),
:deep(.token.tag),
:deep(.token.boolean),
:deep(.token.number),
:deep(.token.constant),
:deep(.token.symbol) {
  color: #79c0ff;
}

:deep(.token.selector),
:deep(.token.attr-name),
:deep(.token.string),
:deep(.token.char),
:deep(.token.builtin) {
  color: #a5d6ff;
}

:deep(.token.operator),
:deep(.token.entity),
:deep(.token.url),
:deep(.language-css .token.string),
:deep(.style .token.string),
:deep(.token.variable) {
  color: #ff7b72;
}

:deep(.token.atrule),
:deep(.token.attr-value),
:deep(.token.keyword) {
  color: #ffa657;
}

:deep(.token.regex),
:deep(.token.important) {
  color: #d2a8ff;
}
</style>
