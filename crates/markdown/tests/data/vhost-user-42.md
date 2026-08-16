## vhost-user

在 vhost\_net 的方案中，由于 vhost\_net 实现在内核中，guest 与 vhost\_net 的通信，相较于原生的 virtio 方式性能上有了一定程度的提升，从 guest 到 kvm.ko 的交互只有一次用户态的切换以及数据拷贝。这个方案对于不同 host 之间的通信，或者 guest 到 host nic 之间的通信是比较好的，但是对于某些用户态进程间的通信，比如数据面的通信方案，openvswitch 和与之类似的 SDN 的解决方案，guest 需要和 host 用户态的 vswitch 进行数据交换，如果采用 vhost\_net 的方案，guest 和 host 之间又存在多次的上下文切换和数据拷贝，为了避免这种情况，业界就想出将 vhost\_net从内核态移到用户态。这就是 vhost-user 的实现。

### &#x20;vhost-user 实现简述

vhost-user 和 vhost\_net 的实现原理是一样，都是采用 vring 完成共享内存，eventfd 机制完成事件通知。不同在于 vhost\_net 实现在内核中，而 vhost-user 实现在用户空间中，用于用户空间中两个进程之间的通信，其采用共享内存的通信方式。

![](../assets/image_1711530902773.png)

vhost-user 基于 C/S 的模式，采用 UNIX 域套接字（UNIX domain socket）来完成进程间的事件通知和数据交互，相比 vhost\_net 中采用 ioctl 的方式，vhost-user 采用 socket 的方式大大简化了操作。

vhost-user 基于 vring 这套通用的共享内存通信方案，只要 client 和 server 按照 vring 提供的接口实现所需功能即可，常见的实现方案是 client 实现在 guest OS 中，一般是集成在 virtio 驱动上，server 端实现在 qemu 中，也可以实现在各种数据面中，如 OVS，Snabbswitch 等虚拟交换机。

如果使用 qemu 作为 vhost-user 的 server 端实现，在启动 qemu 时，我们需要指定 -mem-path 和 -netdev 参数，如：

```
$ qemu -m 1024 -mem-path /hugetlbfs,prealloc=on,share=on
-netdev type=vhost-user,id=net0,file=/path/to/socket
-device virtio-net-pci,netdev=net0
```

指定 -mem-path 意味着 qemu 会在 guest OS 的内存中创建一个文件，share=on 选项允许其他进程访问这个文件，也就意味着能访问 guest OS 内存，达到共享内存的目的。

\-netdev type=vhost-user 指定通信方案，file=/path/to/socket 指定 socket 文件。

当 qemu 启动之后，首先会进行 vring 的初始化，并通过 socket 建立 C/S 的共享内存区域和事件机制，然后 client 通过 eventfd 将 virtio kick 事件通知到 server 端，server 端同样通过 eventfd 进行响应，完成整个数据交互。

![](../assets/image_1711530943727.png)

## vhost-user使用DPDK加速

DPDK社区一直致力于加速数据中心的网络数据平面，而virtio网络作为当今云环境下数据平面必不可少的一环，自然是DPDK优化的方向。而vhost-user就是结合DPDK的各方面优化技术得到的用户态virtio网络后端驱动方案。这些优化技术包括：处理器亲和性、大页内存的使用、轮询模式驱动等。除了vhost-user，DPDK还有自己的virtio PMD作为高性能的前端，本文将以vhost-user作为重点介绍。

基于vhost协议，DPDK设计了一套新的用户态协议，名为vhost-user协议，这套协议允许qemu将virtio设备的网络包处理offload到任何DPDK应用中（例如OVS-DPDK）。vhost-user协议和vhost协议最大的区别其实就是通信信道的区别。Vhost协议通过对vhost-net字符设备进行ioctl实现，而vhost-user协议则通过unix socket进行实现。通过这个unix socket，vhost-user协议允许QEMU通过以下重要的操作来配置数据平面的offload：

1. 特性协商：virtio的特性与vhost-user新定义的特性都可以通过类似的方式协商，而所谓协商的具体实现就是QEMU接收vhost-user的特性，与自己支持的特性取交集。
2. 内存区域配置：QEMU配置好内存映射区域，vhost-user使用mmap接口来映射它们。
3. Vring配置：QEMU将Virtqueue的个数与地址发送给vhost-user，以便vhost-user访问。
4. 通知配置：vhost-user仍然使用eventfd来实现前后端通知。

        基于DPDK的Open vSwitch(OVS-DPDK)一直以来就对vhost-user提供了支持，读者可以通过在OVS-DPDK上创建vhost-user端口来使用这种高效的用户态后端。
