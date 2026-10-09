#!/usr/bin/env escript
%%! -noshell

main(_Args) ->
    {ok, _Applications} = application:ensure_all_started(ssh),
    Options = [
        {system_dir, "/etc/portmate-erlang-ssh"},
        {user_dir, "/home/portmate"},
        {auth_methods, "password"},
        {user_passwords, [{"portmate", "portmate"}]},
        {subsystems, [
            ssh_sftpd:subsystem_spec([
                {cwd, "/home/portmate"},
                {root, "/"}
            ])
        ]}
    ],
    case ssh:daemon(22, Options) of
        {ok, _Daemon} ->
            io:format(
                "PortMate Erlang/OTP ~s SFTP compatibility server is listening~n",
                [erlang:system_info(otp_release)]
            ),
            receive
                stop -> ok
            end;
        {error, Reason} ->
            io:format(standard_error, "failed to start Erlang SFTP server: ~p~n", [Reason]),
            halt(1)
    end.
