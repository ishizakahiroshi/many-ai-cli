package hub

import (
	"fmt"
	"log/slog"
	"runtime/debug"
)

func (s *Server) safeGo(name string, fn func()) {
	go func() {
		defer func() {
			if r := recover(); r != nil {
				logger := s.logger
				if logger == nil {
					logger = slog.Default()
				}
				logger.Error("background goroutine panic recovered",
					"name", name,
					"recover", fmt.Sprintf("%v", r),
					"stack", string(debug.Stack()))
			}
		}()
		fn()
	}()
}
