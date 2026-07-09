from krita import DockWidgetFactory, DockWidgetFactoryBase, Krita

from .vc_docker import VcDocker

Krita.instance().addDockWidgetFactory(
    DockWidgetFactory("kritaVcDocker", DockWidgetFactoryBase.DockRight, VcDocker)
)
