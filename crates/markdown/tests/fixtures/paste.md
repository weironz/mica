如果你用 AI Agent（Claude Code、Cursor、Windsurf、Codex CLI 等）开发完一个项目，后续想做全面代码审查（Code Review）+ 架构优化 + 性能优化 + 安全审计，不要只问"帮我优化代码"，效果会比较差。
比较好的方式是让 AI Agent 扮演不同角色，分阶段审查，并结合一些专用工具/MCP。
下面给你一套比较实用的方法。
一、推荐的整体 Review 流程
建议分 6 个阶段：

```
阶段1：项目理解
        ↓
阶段2：架构审查
        ↓
阶段3：代码质量审查
        ↓
阶段4：性能优化
        ↓
阶段5：安全审计
        ↓
阶段6：工程化提升

```

不要一次性让 Agent 改全部代码，否则容易破坏已有逻辑。
阶段1：让 AI 先理解项目
Prompt：

```
你现在作为一个资深软件架构师。

请先不要修改任何代码，只分析当前项目。

任务：
1. 阅读项目目录结构
2. 分析技术栈

输出：
- 项目整体架构说明
- 关键风险点

要求：
不要猜测，不确定的信息标记出来。

```

如果支持 Mermaid：
让它输出：

```
请使用 Mermaid 绘制：
- 系统架构图
- 模块依赖图
- 数据流图

```

阶段2：架构 Review Prompt
让 AI 变成架构师：

```
你是一名拥有10年以上经验的Principal Engineer。

请审查当前项目架构。

重点检查：

## 架构
- 是否符合SOLID原则
- 模块边界是否合理

## 可维护性
- 后续增加功能是否容易

输出：
1. 架构问题列表
2. 严重等级(P0/P1/P2)

```

阶段3：代码质量 Review
Prompt：

```
你现在作为 Staff Software Engineer。

请对代码进行全面Code Review。

检查：

### 可读性
- 命名
- 函数长度

### Bug风险
寻找：
- 空指针
- 边界条件

输出：
每个问题包含：

文件:
行号:
问题:
风险:
修改建议:

```

推荐搭配工具 / MCP / Skills
1. Claude Code（强烈推荐）
如果你用 Claude Code：
它本身很适合大型 repo review。
推荐安装：
GitHub MCP
用途：

* 查看 PR
* Issue
* Commit历史

例如：

```
Review this repository like a senior maintainer.
Analyze recent commits and identify regression risks.

```

3. SonarQube / SonarCloud
非常推荐。
AI + 静态扫描组合：

```
AI Agent
    |
    |
SonarQube
    |
    |
代码质量报告

```

检查：

* Bug
* Code Smell
* Security Hotspot
* Coverage

5. Snyk
依赖安全：
检查：

```
package.json
requirements.txt
go.mod
pom.xml

```

找：

* CVE
* License风险

如果使用 Claude Code，可以做一个 review skill
例如：

```
.claude/
 └── skills/
      └── code-review/
           └── SKILL.md

```

内容：

```markdown
# Code Review Skill

You are a senior software engineer.

Always review:

1. Architecture
2. Security
3. Performance

Never directly modify code before creating a report.

First:
- analyze
- report
- propose changes

Then wait for approval.

```

以后：

```
/code-review

```

即可。
一个非常强的总 Prompt
可以直接给 Claude Code / Cursor：

```
Act as a Principal Engineer from a top-tier software company.

Perform a complete engineering review of this repository.

Do NOT modify code initially.

Analyze:

1. Architecture
2. Code quality

For each issue provide:

- Severity: Critical/High/Medium/Low
- Location

After analysis, create:

1. Executive summary
2. Technical debt list

Only after approval, start implementing fixes.

```

结合你的使用场景，我建议你的流程更偏企业级项目审查：

```
Claude Code
    +
SonarQube
    +
Semgrep
    +
GitHub Actions
    +
Docker Scout
    +
Trivy

```

这套基本接近中大型公司的代码质量流水线。
