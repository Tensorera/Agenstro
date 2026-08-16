# 设置开发环境

本指南分别创建 Clef SDK、Tactus Runtime、Segno Flow 和 Motivo Studio
开发环境，并运行本地质量检查。

## 1. 项目边界

仓库包含三个独立安装的 Python 项目和一个独立前端项目：

- Clef SDK
  - 源码：`clef-sdk/src/clef_sdk/`
  - 测试：`clef-sdk/tests/`
  - 项目元数据：`clef-sdk/pyproject.toml`
- Tactus Runtime
  - 源码：`tactus-runtime/src/tactus_runtime/`
  - 测试：`tactus-runtime/tests/`
  - 项目元数据：`tactus-runtime/pyproject.toml`
- Segno Flow
  - 源码：`segno-flow/src/segno_flow/`
  - 测试：`segno-flow/tests/`
  - 项目元数据：`segno-flow/pyproject.toml`
- Motivo Studio
  - 前端：`motivo-studio/`

各项目维护自己的元数据、虚拟环境和依赖；不要在仓库根目录重新引入
共享 `src/` 或共享运行状态。

## 2. Clef SDK

从仓库根目录创建环境并安装开发依赖：

```powershell
py -3.12 -m venv clef-sdk\.venv
.\clef-sdk\.venv\Scripts\python -m pip install -e ".\clef-sdk[dev]" build
```

运行本地检查：

```powershell
.\clef-sdk\.venv\Scripts\python -m ruff check clef-sdk/src clef-sdk/tests
.\clef-sdk\.venv\Scripts\python -m ruff format --check clef-sdk/src clef-sdk/tests
.\clef-sdk\.venv\Scripts\python -m pytest clef-sdk/tests
.\clef-sdk\.venv\Scripts\python -m build clef-sdk
.\clef-sdk\.venv\Scripts\python -m mkdocs build --strict
```

## 3. Tactus Runtime

Tactus 只支持 Windows 与 CPython 3.12。使用独立环境，并用约束文件
冻结 Windows 依赖解析：

```powershell
py -3.12 -m venv tactus-runtime\.venv
.\tactus-runtime\.venv\Scripts\python -m pip install `
  -c .\tactus-runtime\constraints-windows-py312.txt `
  -e ".\tactus-runtime[dev]" build
```

在 Tactus 项目根运行它自己的检查：

```powershell
Push-Location tactus-runtime
.\.venv\Scripts\python -m ruff check src tests
.\.venv\Scripts\python -m ruff format --check src tests
.\.venv\Scripts\python -m pytest
.\.venv\Scripts\python -m build .
Pop-Location
```

## 4. Segno Flow

```powershell
py -3.12 -m venv segno-flow\.venv
.\segno-flow\.venv\Scripts\python -m pip install -e ".\segno-flow[dev,ui]" build
Push-Location segno-flow
.\.venv\Scripts\python -m ruff check src tests
.\.venv\Scripts\python -m pytest
Pop-Location
```

## 5. Motivo Studio 与前端

分别在 `motivo-studio/` 和 `segno-flow/frontend/` 执行：

```powershell
npm ci
npm test
npm run typecheck
npm run build
```

## 6. 命名与格式

- 模块、函数和变量使用 `snake_case`。
- 类使用 `PascalCase`。
- 常量使用 `UPPER_CASE`。
- 异常类使用 `Error` 后缀。
- 包的公开名称通过 `__all__` 明确导出。
- Ruff 是唯一的 Python formatter 和 lint 配置来源。
- 行宽为 88 个字符，目标版本为 Python 3.12。

先运行 `ruff check --fix` 处理安全的机械修复，再运行 `ruff format`。
需要改变控制流或公共名称的告警应单独审查，不使用批量 unsafe fix。

## 7. Docstring 与文档

公开模块、类、函数和方法遵循 PEP 257。首行用一句祈使句概括行为。
需要补充时，在摘要后空一行再写参数约束、副作用或异常。测试函数可以省略
docstring。

公共 API、配置字段、CLI、状态转换或目录边界变化时，应先更新对应 Reference，
再更新受影响的 Tutorial、How-to 或 Explanation。仓库级 `archive/` 是历史
资料，不得用来证明当前行为。
