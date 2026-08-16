# 已归档 — 原始构想

> **Status: archived.** 本页原样保存项目立项时的输入，不描述当前实现。
> 当前行为只以 `docs/` 下的现行文档和源码为准。

---

这个目录目前完全是空的，我打算写一个基于opencode的agent clef框架，我来跟你讲一讲。


这个agent框架解决两个问题：


- 一个固定的workflow的可复现性问题。
- 复杂agent workflow的编译问题
- subagent的调度与协调问题。

这个框架的思路是什么呢？

把agent运行封装到一个带有副作用的函数（称作领域函数，domain function），以及对应的数据结构，通过一般的程序语言（例如python）调用这些函数，从而能够设计出非常复杂的workflow。

这带来了两个好处：

1. 我们可以借助程序语言在一定程度上评估和控制大模型的输出结果（甚至用另一个领域函数去评估！）
2. 我们的每一个skills，都能编写为具有确定性的脚本！

以下是我的一些基本思路，请你详细阅读之后补充或者讨论其中的一些内容，放置在D:\VibeWorkspace\Agentro\docs\concept的markdown中（你可以创建一个新的）

我们要写几层内容：

1. 程序语言到opencode agent的信息传递协议。
2. 领域函数与领域对象的算法与数据结构
3. 基本的领域函数。

这三部分我一个一个写，首先是程序语言到opencode agnet的协议

我早期看过opencode agent的结构，其实主要是以session作为基本单元进行agent loop的，他的输入和输出主要是纯文本，然后支持对session中进行各种操作，这是我们的基础。

但除了管理session其实不太够，我们需要opencode的session能够提供一些额外的信息：

- 任务运行中产生或者清除的文件或这文本，这些我们统称为artifact
- 任务运行是否受阻，是否存在一些障碍，也就是错误状态与错误原因

这些我们需要opencode的session能够返回一个规定好框架的AST，大致的字段包括：

- Text
- Artifact list，这个list包括每个artifact的文件路径以及作用。
- run states
- Error information

这个AST需要大模型自己去写，所以是有副作用的。

第二部分是领域函数的领域对象的数据结构，以及对应的一些算法。

这一部分我还没有太想好。但我的思路是遵循基本的结构，这些都需要有。
I  Input            输入类型
O  Output           输出类型
P  Preconditions    前置条件
Q  Postconditions   后置条件
E  Effects          允许的副作用
R  Resources        资源和执行策略
V  Verifier         结果验证器

接下来是基本数据结构和基本函数，我们至少需要这些数据结构

- Artifact，这是一个由路径+对应的描述组成的数据结构，他是传入传出的最基本的数据结构
- prompt，代表额外注入进去的指令与内容。
- context，代表大模型执行任务返送回来的context。
- SessionTask，一个聚合了输入artifact，期望输出artifact以及任务prompt列表（因为每一个任务可能有若干个prompt组成，允许chain of thought）的集合，是session运行的最小单元。
- sessionResult，聚合了输出artifact，运行状态与可能的error information，SessionTask（因为每个result至少要包含一个输入的sessionTask）
- WorkflowPlan，这是一个有向图，其中sessionTask作为节点，SessionResult作为有向边，它们之间的关系是，sessionTask的artifact由汇入其中的sessionResult组成（但也可以额外增加和定义）。
与之对应的一系列基本函数是：

sessionResult=DomainRun(sessionTask)，执行任务，这个是最简单的，执行单次任务。

这里传递到opencode的时候，实际上是将输入Artifact和输出artifact都注入prompt当中，随着指令的prompt发送给opencode，等待运行结束。

List<sessionResult>=ExcutePlan(WorkflowPlan)

这个ExcutePlan按照有向图的方向，逐个执行任务，收集artifact，组装到下一个节点，执行...直到最终节点，当然有可能有很多最终节点。

WorkflowPlan=createPlan(sessionTask)，这个可以先不写，因为比较困难。


以上就是我的初步想法，请你先把我的输入原封不动保存到文件夹中，然后再写你增补的框架。

- 语言，推荐使用python，将其写成一个模组的形式，能够利用import导入，运行时直接可以用python script.py的形式运行，目标框架还是3.12吧（就本机安装的版本即可）
- 配置文件，建议使用User/名称/profiles的形式，在写的script当中首先需要导入profiles，然后才能运行，然后每次运行的时候将profile作为依赖注入项注入到函数中。
- opencode的鉴权问题，目前可以采用简单的方案：传递工作文件夹字段，在工作文件夹内部允许文件删除、修改、移动等。开放read权限。其余bash和powershell命令暂时均可允许。
